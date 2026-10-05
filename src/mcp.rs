//! Real MCP transport for a host-owned Botus document service.
//!
//! The listener and call capability registry live inside the MobKit host. The
//! stdio executable is only a relay: its credential authenticates that relay,
//! never an agent. Every tool call also needs a short-lived, one-use capability
//! issued from the trusted runtime dispatch context for the exact destination,
//! operation and arguments. Workbook authorization remains in `native`.

mod capability;
mod transport;

use crate::{
    Limits,
    native::{BotusToolFactory, MAX_REQUEST_BYTES, tool_definitions},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use meerkat_core::{
    AgentToolDispatcher, CALL_ORIGIN_META_KEY, McpServerConfig, ToolCallView, ToolDispatchContext,
    ToolMutationClass, WireCallOrigin,
};
use meerkat_mcp::{McpCallContext, McpCallContextError, McpCallContextProvider, McpCallTarget};
use meerkat_mobkit::extension_state::{
    ChildMcpAvailability, HostMcpDocumentBinding, ToolBundleContext, ToolBundleFactory,
};
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
        ClientInfo, ClientRequest, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, ServerJsonRpcMessage, ServerResult,
        Tool, ToolAnnotations,
    },
    service::{Peer, PeerRequestOptions, RequestContext, ServiceError},
    transport::{
        StreamableHttpClientTransport,
        async_rw::JsonRpcMessageCodec,
        streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use serde_json::Value;
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

pub use capability::{CallLease, PreparedCall};
use capability::{CallRegistry, random_secret};

/// Host-owned metadata. It contains no self-asserted actor or ancestry.
pub const CALL_META_KEY: &str = "ai.rkat.botus/call";
pub const ENDPOINT_ENV: &str = "BOTUS_MCP_ENDPOINT";
pub const RELAY_TOKEN_ENV: &str = "BOTUS_MCP_RELAY_TOKEN";
const MCP_PATH: &str = "/mcp";
const MAX_HTTP_BYTES: usize = MAX_REQUEST_BYTES + 16 * 1024;
pub const MCP_RESPONSE_RESERVE_BYTES: usize = 256;
pub const MIN_MCP_RESPONSE_BYTES: usize = 16 * 1024 + MCP_RESPONSE_RESERVE_BYTES;

/// Register this same object as the durable document factory and keep its
/// listener alive for the runtime's lifetime. The factory binds exactly once.
pub struct BotusMcpHost {
    factory: BotusToolFactory,
    dispatcher: OnceLock<Arc<dyn AgentToolDispatcher>>,
    registry: Arc<CallRegistry>,
    started: AtomicBool,
    max_response_bytes: usize,
}

impl Default for BotusMcpHost {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl BotusMcpHost {
    /// Binding rejects response limits below `MIN_MCP_RESPONSE_BYTES`. The
    /// transport reserve is deducted before native mutation preflight runs.
    pub fn new(mut limits: Limits) -> Self {
        let max_response_bytes = limits.max_response_bytes;
        limits.max_response_bytes = limits
            .max_response_bytes
            .saturating_sub(MCP_RESPONSE_RESERVE_BYTES);
        Self {
            factory: BotusToolFactory { limits },
            dispatcher: OnceLock::new(),
            registry: Arc::new(CallRegistry::default()),
            started: AtomicBool::new(false),
            max_response_bytes,
        }
    }

    /// Bind only IPv4 loopback, on an OS-assigned port. No public listener or
    /// unprotected remote transport is exposed by this package.
    pub async fn start(self: &Arc<Self>) -> io::Result<BotusMcpListener> {
        if self.started.swap(true, Ordering::AcqRel) {
            return Err(io::Error::other("Botus MCP host has already started"));
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let token = random_secret()?;
        let endpoint = format!("http://{address}{MCP_PATH}");
        let cancellation = CancellationToken::new();
        let mut config = StreamableHttpServerConfig::default();
        // A session carries MCP cancellation notifications to the in-flight
        // request. It never supplies caller identity or workbook authority.
        config.stateful_mode = true;
        config.allowed_hosts = vec![address.to_string()];
        config.cancellation_token = cancellation.clone();
        let host = Arc::clone(self);
        let service = StreamableHttpService::new(
            move || {
                Ok(HostHandler {
                    host: Arc::clone(&host),
                })
            },
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let auth = Arc::new(format!("Bearer {token}"));
        let router = Router::new()
            .nest_service(MCP_PATH, service)
            .layer(middleware::from_fn(move |request, next| {
                authenticate(Arc::clone(&auth), request, next)
            }));
        let shutdown = cancellation.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        });
        Ok(BotusMcpListener {
            host: Arc::clone(self),
            endpoint,
            relay_token: token,
            cancellation,
            task: Some(task),
        })
    }
}

impl ToolBundleFactory for BotusMcpHost {
    fn build(&self, context: ToolBundleContext) -> Result<Arc<dyn AgentToolDispatcher>, String> {
        if self.max_response_bytes < MIN_MCP_RESPONSE_BYTES {
            return Err(format!(
                "Botus MCP response limit must be at least {MIN_MCP_RESPONSE_BYTES} bytes"
            ));
        }
        let dispatcher = self.factory.build(context)?;
        self.dispatcher
            .set(Arc::clone(&dispatcher))
            .map_err(|_| "Botus MCP host document context is already bound".to_owned())?;
        Ok(dispatcher)
    }
}

/// Protected launch configuration and listener lifetime. Debug deliberately
/// omits the relay credential. Dropping this handle stops the endpoint.
pub struct BotusMcpListener {
    host: Arc<BotusMcpHost>,
    endpoint: String,
    relay_token: String,
    cancellation: CancellationToken,
    task: Option<tokio::task::JoinHandle<io::Result<()>>>,
}

impl std::fmt::Debug for BotusMcpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotusMcpListener")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl BotusMcpListener {
    /// Register the host service without persisting this process's endpoint
    /// or relay credential. Profiles select the returned public descriptor;
    /// MobKit resolves its declared environment slots at each agent build.
    /// Child availability exposes the tools, never a workbook access grant.
    pub fn document_binding(
        &self,
        command: impl Into<String>,
        children: ChildMcpAvailability,
    ) -> io::Result<HostMcpDocumentBinding> {
        let runtime = self.relay_config(command);
        let provider = Arc::new(self.call_context_provider(runtime.clone())?);
        let mut public = runtime.clone();
        let meerkat_core::mcp_config::McpTransportConfig::Stdio(stdio) = &mut public.transport
        else {
            return Err(io::Error::other("Invalid Botus relay transport"));
        };
        for value in stdio.env.values_mut() {
            value.clear();
        }
        HostMcpDocumentBinding::new(
            public,
            runtime,
            BotusToolFactory::requirements(),
            self.host.clone(),
            provider,
            children,
        )
        .map_err(io::Error::other)
    }

    /// The host passes this configuration directly to its MCP router. Do not
    /// put the resulting environment map in profiles, prompts, logs or source
    /// control. Prefer `document_binding` for automatic MobKit composition.
    pub fn relay_config(&self, command: impl Into<String>) -> McpServerConfig {
        McpServerConfig::stdio(
            "botus-1-2-3",
            command,
            vec!["mcp".into()],
            HashMap::from([
                (ENDPOINT_ENV.into(), self.endpoint.clone()),
                (RELAY_TOKEN_ENV.into(), self.relay_token.clone()),
            ]),
        )
    }

    /// Bind to a complete configuration, including command, environment and
    /// tool mappings. A same-name configuration cannot receive capabilities.
    pub fn call_context_provider(
        &self,
        config: McpServerConfig,
    ) -> io::Result<BotusMcpContextProvider> {
        let meerkat_core::mcp_config::McpTransportConfig::Stdio(stdio) = &config.transport else {
            return Err(io::Error::other(
                "Botus requires its protected stdio relay configuration",
            ));
        };
        if stdio.env.get(ENDPOINT_ENV) != Some(&self.endpoint)
            || stdio.env.get(RELAY_TOKEN_ENV) != Some(&self.relay_token)
            || stdio.args != ["mcp"]
            || !config.tool_names.is_empty()
        {
            return Err(io::Error::other(
                "Botus requires this listener's exact relay and canonical tool names",
            ));
        }
        Ok(BotusMcpContextProvider {
            config,
            registry: Arc::clone(&self.host.registry),
        })
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        self.cancellation.cancel();
        self.host.registry.close();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(result) => result.map_err(io::Error::other)??,
                Err(_) => {
                    task.abort();
                    return Err(io::Error::other("Botus MCP shutdown timed out"));
                }
            }
        }
        Ok(())
    }
}

impl Drop for BotusMcpListener {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.host.registry.close();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// A host-only destination-bound issuer. Its input must come from the native
/// router's dispatch context, never JSON tool arguments or model claims.
pub struct BotusMcpContextProvider {
    config: McpServerConfig,
    registry: Arc<CallRegistry>,
}

impl BotusMcpContextProvider {
    /// Prepare one call. Unrelated destinations are untouched; the selected
    /// destination fails closed when runtime session identity is unavailable.
    pub fn prepare(
        &self,
        config: &McpServerConfig,
        connection_id: &str,
        raw_operation: &str,
        arguments: &serde_json::value::RawValue,
        origin: &WireCallOrigin,
        context: &ToolDispatchContext,
    ) -> io::Result<Option<PreparedCall>> {
        if config != &self.config {
            return Ok(None);
        }
        self.registry
            .issue(connection_id, raw_operation, arguments, origin, context)
            .map(Some)
    }
}

#[async_trait::async_trait]
impl McpCallContextProvider for BotusMcpContextProvider {
    async fn prepare(
        &self,
        target: McpCallTarget<'_>,
        call: ToolCallView<'_>,
        context: &ToolDispatchContext,
    ) -> Result<Option<McpCallContext>, McpCallContextError> {
        self.prepare(
            target.config,
            &target.connection_id.to_string(),
            target.raw_operation,
            call.args,
            target.origin,
            context,
        )
        .map(|prepared| prepared.map(|call| McpCallContext::new(call.metadata, call.lease)))
        .map_err(|_| McpCallContextError::Unavailable)
    }

    fn tool_mutation_class(
        &self,
        config: &McpServerConfig,
        raw_operation: &str,
    ) -> ToolMutationClass {
        if config != &self.config {
            return ToolMutationClass::Unknown;
        }
        match raw_operation {
            "botus_read" => ToolMutationClass::ReadOnly,
            "botus_apply" => ToolMutationClass::Mutating,
            _ => ToolMutationClass::Unknown,
        }
    }
}

async fn authenticate(auth: Arc<String>, mut request: Request, next: Next) -> Response {
    if let Some(value) = request
        .headers_mut()
        .get_mut(axum::http::header::AUTHORIZATION)
    {
        value.set_sensitive(true);
    }
    let supplied = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    if !bool::from(supplied.ct_eq(auth.as_bytes()))
        || request.headers().contains_key(axum::http::header::ORIGIN)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (mut parts, body) = request.into_parts();
    let bytes =
        match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, MAX_HTTP_BYTES)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        };
    // rmcp DEBUG logging includes request metadata. Keep the live capability
    // in opaque HTTP extensions before its protocol service sees the request.
    // Malformed input never reaches rmcp's payload-printing parse diagnostics.
    parts.headers.remove(axum::http::header::AUTHORIZATION);
    if parts.method == axum::http::Method::POST {
        let mut message: Value = match serde_json::from_slice(&bytes) {
            Ok(message) => message,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        if let Some(metadata) = message
            .pointer_mut("/params/_meta")
            .and_then(Value::as_object_mut)
        {
            if let Some(capability) = metadata.remove(CALL_META_KEY) {
                parts.extensions.insert(ProtectedCapability(capability));
            }
        }
        let bytes = match serde_json::to_vec(&message) {
            Ok(bytes) => bytes,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        next.run(Request::from_parts(parts, Body::from(bytes)))
            .await
    } else {
        next.run(Request::from_parts(parts, Body::from(bytes)))
            .await
    }
}

#[derive(Clone)]
struct ProtectedCapability(Value);

impl std::fmt::Debug for ProtectedCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProtectedCapability(<redacted>)")
    }
}

fn server_info() -> ServerInfo {
    let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
    info.server_info = Implementation::new("botus-1-2-3", env!("CARGO_PKG_VERSION"));
    info
}

fn mcp_tools() -> Result<ListToolsResult, ErrorData> {
    let tools = tool_definitions()
        .into_iter()
        .map(|definition| {
            let schema = definition
                .input_schema
                .as_object()
                .cloned()
                .ok_or_else(|| ErrorData::internal_error("Invalid Botus tool schema", None))?;
            let read = definition.name == "botus_read";
            Ok(
                Tool::new(definition.name.to_string(), definition.description, schema)
                    .with_annotations(
                        ToolAnnotations::new()
                            .read_only(read)
                            .destructive(!read)
                            .idempotent(true)
                            .open_world(false),
                    ),
            )
        })
        .collect::<Result<Vec<_>, ErrorData>>()?;
    let mut result = ListToolsResult::default();
    result.tools = tools;
    Ok(result)
}

#[derive(Clone)]
struct HostHandler {
    host: Arc<BotusMcpHost>,
}

impl ServerHandler for HostHandler {
    fn get_info(&self) -> ServerInfo {
        server_info()
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        mcp_tools()
            .ok()?
            .tools
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn initialize(
        &self,
        request: ClientInfo,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerInfo, ErrorData> {
        if self.host.dispatcher.get().is_none() {
            return Err(ErrorData::internal_error(
                "Botus host document services are not bound",
                None,
            ));
        }
        context.peer.set_peer_info(request);
        Ok(server_info())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|page| page.cursor.is_some()) {
            return Err(ErrorData::invalid_params(
                "Botus tool list has no continuation cursor",
                None,
            ));
        }
        mcp_tools()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if request.task.is_some() {
            return Err(ErrorData::invalid_params(
                "Botus does not support task-augmented calls",
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let capability = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ProtectedCapability>());
        let claim = self
            .host
            .registry
            .consume(
                capability.map(|capability| &capability.0),
                context.meta.0.get(CALL_ORIGIN_META_KEY),
                request.name.as_ref(),
                &arguments,
            )
            .map_err(|_| ErrorData::invalid_request("Botus call authority unavailable", None))?;
        let dispatcher = self.host.dispatcher.get().ok_or_else(|| {
            ErrorData::internal_error("Botus host document services are not bound", None)
        })?;
        let raw = serde_json::value::to_raw_value(&arguments)
            .map_err(|_| ErrorData::invalid_params("Invalid Botus arguments", None))?;
        let call_id = context.id.to_string();
        let call = ToolCallView {
            id: &call_id,
            name: request.name.as_ref(),
            args: &raw,
        };
        let outcome = tokio::select! {
            biased;
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("Botus call cancelled", None)),
            _ = claim.cancelled.cancelled() => return Err(ErrorData::internal_error("Botus call lease ended", None)),
            _ = self.host.registry.shutdown.cancelled() => return Err(ErrorData::internal_error("Botus host stopped", None)),
            outcome = dispatcher.dispatch_with_context(call, &claim.context) => outcome,
        }.map_err(|_| ErrorData::internal_error("Botus caller authority or host service unavailable", None))?;
        mcp_result(outcome.result, self.host.max_response_bytes)
    }
}

fn mcp_result(
    result: meerkat_core::ToolResult,
    max_response_bytes: usize,
) -> Result<CallToolResult, ErrorData> {
    let content = vec![Content::text(result.text_content())];
    let result = if result.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    // Native preflight has already reserved more than this fixed envelope's
    // overhead before committing. This guards against future codec changes;
    // it is not the operational oversized-selection path.
    let bytes = serde_json::to_vec(&result)
        .map_err(|_| ErrorData::internal_error("Cannot encode Botus MCP result", None))?;
    if bytes.len() > max_response_bytes {
        return Err(ErrorData::internal_error(
            "Botus MCP result envelope invariant failed",
            None,
        ));
    }
    Ok(result)
}

#[derive(Clone)]
struct RelayHandler {
    upstream: Peer<RoleClient>,
}

impl ServerHandler for RelayHandler {
    fn get_info(&self) -> ServerInfo {
        server_info()
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        mcp_tools()
            .ok()?
            .tools
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.upstream.list_tools(request).await.map_err(relay_error)
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // rmcp extracts request metadata into RequestContext on ingress.
        // Forward it intact, including the canonical origin codec when present.
        if request.meta.is_none() && !context.meta.0.is_empty() {
            request.meta = Some(context.meta.clone());
        }
        let mut outgoing = CallToolRequest::new(request);
        if let Some(capability) = context.extensions.get::<ProtectedCapability>() {
            outgoing.extensions.insert(capability.clone());
        }
        let handle = self
            .upstream
            .send_request_with_option(
                ClientRequest::CallToolRequest(outgoing),
                PeerRequestOptions::with_timeout(Duration::from_secs(60)),
            )
            .await
            .map_err(relay_error)?;
        let request_id = handle.id.clone();
        tokio::select! {
            biased;
            _ = context.ct.cancelled() => {
                let _ = self.upstream.notify_cancelled(CancelledNotificationParam { request_id, reason: Some("Botus downstream cancelled".into()) }).await;
                Err(ErrorData::internal_error("Botus call cancelled", None))
            },
            result = handle.await_response() => match result.map_err(relay_error)? {
                ServerResult::CallToolResult(result) => Ok(result),
                _ => Err(ErrorData::internal_error("Unexpected Botus host response", None)),
            },
        }
    }
}

fn relay_error(error: ServiceError) -> ErrorData {
    match error {
        ServiceError::McpError(error) => error,
        _ => ErrorData::internal_error("Botus host connection unavailable", None),
    }
}

/// Start the actual stdio MCP relay. Validate host settings and complete the
/// upstream MCP handshake before accepting an agent connection on stdin.
pub async fn serve_stdio_from_env() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = std::env::var(ENDPOINT_ENV)
        .map_err(|_| "BOTUS_MCP_ENDPOINT is required from the Botus host")?;
    let token = std::env::var(RELAY_TOKEN_ENV)
        .map_err(|_| "BOTUS_MCP_RELAY_TOKEN is required from the Botus host")?;
    serve_stdio(&endpoint, &token).await
}

/// Public for host-controlled launchers. A relay credential alone authorizes
/// no spreadsheet operation: calls still need the native per-call capability.
pub async fn serve_stdio(
    endpoint: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    validate_relay_settings(endpoint, token)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()?;
    let transport = StreamableHttpClientTransport::with_client(
        transport::ProtectedHttpClient(client),
        StreamableHttpClientTransportConfig::with_uri(endpoint.to_owned()).auth_header(token),
    );
    let upstream = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .map_err(|_| "Botus host handshake timed out")??;
    let relay = RelayHandler {
        upstream: upstream.peer().clone(),
    };
    let input = tokio_util::codec::FramedRead::new(
        tokio::io::stdin(),
        transport::ProtectedInputCodec::new(MAX_HTTP_BYTES),
    )
    .take_while(|frame| std::future::ready(frame.is_ok()))
    .filter_map(|frame| std::future::ready(frame.ok()));
    let output = tokio_util::codec::FramedWrite::new(
        tokio::io::stdout(),
        JsonRpcMessageCodec::<ServerJsonRpcMessage>::new_with_max_length(MAX_HTTP_BYTES),
    );
    let downstream = relay.serve((output, input)).await?;
    let result = downstream.waiting().await;
    let _ = upstream.cancel().await;
    result?;
    Ok(())
}

fn validate_relay_settings(endpoint: &str, token: &str) -> io::Result<()> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| io::Error::other("Invalid Botus host endpoint"))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.path() != MCP_PATH
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(io::Error::other(
            "Botus host endpoint must be an explicit IPv4 loopback HTTP /mcp address",
        ));
    }
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid Botus relay credential"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_envelope_reservation_covers_escaped_content_and_request_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        for text in ["", "plain", "\"\\\n\r\t", "Unicode \u{1f4be}"] {
            for id in ["0", "\"\\\n", &"id-".repeat(100)] {
                for is_error in [false, true] {
                    let native =
                        meerkat_core::ToolResult::new(id.into(), text.repeat(8000), is_error);
                    let native_bytes = serde_json::to_vec(&native)?.len();
                    let mcp = mcp_result(native, native_bytes + MCP_RESPONSE_RESERVE_BYTES)?;
                    assert!(
                        serde_json::to_vec(&mcp)?.len()
                            <= native_bytes + MCP_RESPONSE_RESERVE_BYTES
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn relay_endpoint_validation_is_literal_loopback_only() {
        let token = "a".repeat(64);
        assert!(validate_relay_settings("http://127.0.0.1:12345/mcp", &token).is_ok());
        for endpoint in [
            "http://localhost:12345/mcp",
            "http://127.0.0.1.example:12345/mcp",
            "http://127.0.0.1:12345/mcp?next=x",
            "http://user@127.0.0.1:12345/mcp",
            "https://example.com/mcp",
        ] {
            assert!(validate_relay_settings(endpoint, &token).is_err());
        }
        assert!(validate_relay_settings("http://127.0.0.1:12345/mcp", "short").is_err());
    }
}
