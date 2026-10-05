#![cfg(feature = "mcp")]
//! Real stdio child -> HTTP MCP -> original MobKit document service tests.
//! The resolver is explicit test authority; full native lineage belongs in the
//! MobKit host acceptance suite, not in a second identity implementation here.

use async_trait::async_trait;
use botus_1_2_3::{
    mcp::{BotusMcpContextProvider, BotusMcpHost, BotusMcpListener, PreparedCall},
    native::{NAMESPACE, tool_definitions},
};
use meerkat_core::{
    McpServerConfig, SessionId, ToolDispatchContext, mcp_config::McpTransportConfig,
};
use meerkat_mobkit::extension_state::{
    ToolBundleContext, ToolBundleFactory, ToolCallerResolver, documents::*,
};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, Meta},
    service::{PeerRequestOptions, RunningService},
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Authority {
    allowed: AtomicBool,
    blocking: AtomicBool,
    session: SessionId,
    entered: tokio::sync::Notify,
    dropped: AtomicUsize,
}
struct ResolveGuard<'a>(&'a AtomicUsize);
impl Drop for ResolveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[async_trait]
impl ToolCallerResolver for Authority {
    async fn resolve(
        &self,
        context: &ToolDispatchContext,
        _requested_owner: Option<&Owner>,
    ) -> Result<HostAccessContext> {
        let _guard = ResolveGuard(&self.dropped);
        self.entered.notify_one();
        if self.blocking.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if !self.allowed.load(Ordering::SeqCst)
            || context.origin_session_id() != Some(&self.session)
        {
            return Err(Error::AuthorityUnavailable);
        }
        HostAccessContext::new(
            "mcp-test-realm",
            Principal::Agent("same-durable-agent".into()),
            VerifiedLineage::root(),
            HostPolicy::ALLOW,
        )
    }
}

struct Fixture {
    listener: BotusMcpListener,
    config: McpServerConfig,
    provider: BotusMcpContextProvider,
    authority: Arc<Authority>,
    context: ToolDispatchContext,
}
impl Fixture {
    async fn new(path: &Path) -> TestResult<Self> {
        Self::with_limits(path, botus_1_2_3::Limits::default()).await
    }

    async fn with_limits(path: &Path, limits: botus_1_2_3::Limits) -> TestResult<Self> {
        let host = Arc::new(BotusMcpHost::new(limits));
        let listener = host.start().await?;
        let config = listener.relay_config(env!("CARGO_BIN_EXE_botus-1-2-3"));
        let provider = listener.call_context_provider(config.clone())?;
        let session = SessionId::new();
        let authority = Arc::new(Authority {
            allowed: AtomicBool::new(true),
            blocking: AtomicBool::new(false),
            session: session.clone(),
            entered: tokio::sync::Notify::new(),
            dropped: AtomicUsize::new(0),
        });
        let documents = DocumentService::new(
            "mcp-test-realm",
            NAMESPACE,
            Arc::new(SqliteExtensionDocumentStore::open(path)?),
        )?;
        host.build(ToolBundleContext {
            documents,
            caller_resolver: authority.clone(),
        })
        .map_err(std::io::Error::other)?;
        Ok(Self {
            listener,
            config,
            provider,
            authority,
            context: ToolDispatchContext::default().with_runtime_identity(session, None),
        })
    }

    async fn connect(&self) -> TestResult<RunningService<RoleClient, ()>> {
        let McpTransportConfig::Stdio(config) = &self.config.transport else {
            return Err("Expected stdio".into());
        };
        let mut command = tokio::process::Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .kill_on_drop(true);
        Ok(().serve(TokioChildProcess::new(command)?).await?)
    }

    fn request(
        &self,
        name: &str,
        value: Value,
    ) -> TestResult<(CallToolRequestParams, PreparedCall)> {
        let args = serde_json::value::to_raw_value(&value)?;
        let prepared = self
            .provider
            .prepare(
                &self.config,
                "actual-native-connection",
                name,
                &args,
                &self.context,
            )?
            .ok_or("Destination not selected")?;
        let mut request = CallToolRequestParams::new(name.to_owned())
            .with_arguments(value.as_object().cloned().ok_or("Expected object")?);
        request.meta = Some(Meta(prepared.metadata.clone()));
        Ok((request, prepared))
    }
}

fn body(result: &CallToolResult) -> TestResult<Value> {
    Ok(serde_json::from_str(
        &result
            .content
            .first()
            .and_then(|content| content.as_text())
            .ok_or("Expected text")?
            .text,
    )?)
}

#[tokio::test]
async fn stdio_catalog_create_read_currentness_and_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("host.sqlite3");
    let fixture = Fixture::new(&path).await?;
    let client = fixture.connect().await?;
    let catalog = client.list_all_tools().await?;
    assert_eq!(catalog.len(), 2);
    assert!(serde_json::to_vec(&catalog)?.len() <= botus_1_2_3::native::MAX_TOOL_DEFINITION_BYTES);
    for native in tool_definitions() {
        let tool = catalog
            .iter()
            .find(|tool| tool.name == native.name.to_string())
            .ok_or("Missing tool")?;
        assert_eq!(
            serde_json::to_value(&*tool.input_schema)?,
            native.input_schema
        );
        assert_eq!(
            tool.description.as_deref(),
            Some(native.description.as_str())
        );
    }
    assert!(
        client
            .call_tool(
                CallToolRequestParams::new("botus_read").with_arguments(
                    json!({"action":"identity"})
                        .as_object()
                        .cloned()
                        .ok_or("object")?
                )
            )
            .await
            .is_err()
    );
    let (request, _lease) = fixture.request("botus_apply", json!({"action":"create","request_id":"create-once","title":"1-2-3 MCP","operations":[{"op":"put","at":"A1","values":[[7,{"formula":"=A1*3"}]]}]}))?;
    let result = client.call_tool(request).await?;
    assert_eq!(result.is_error, Some(false));
    let created = body(&result)?;
    let id = created["receipt"]["document_id"]
        .as_str()
        .ok_or("Missing document ID")?
        .to_owned();
    let (request, _lease) = fixture.request(
        "botus_read",
        json!({"action":"read","id":id,"query":{"range":"A1:B1"}}),
    )?;
    let old_request = request.clone();
    let result = client.call_tool(request).await?;
    assert_eq!(result.is_error, Some(false));
    assert!(body(&result)?.to_string().contains("21"));
    assert!(client.call_tool(old_request.clone()).await.is_err());
    let (request, _lease) = fixture.request("botus_read", json!({"action":"identity"}))?;
    fixture.authority.allowed.store(false, Ordering::SeqCst);
    assert!(client.call_tool(request).await.is_err());
    client.cancel().await?;
    fixture.listener.shutdown().await?;
    let reopened = Fixture::new(&path).await?;
    let client = reopened.connect().await?;
    assert!(client.call_tool(old_request).await.is_err());
    let (request, _lease) = reopened.request(
        "botus_read",
        json!({"action":"read","id":id,"query":{"range":"A1:B1"}}),
    )?;
    let result = client.call_tool(request).await?;
    assert_eq!(result.is_error, Some(false));
    assert!(body(&result)?.to_string().contains("21"));
    client.cancel().await?;
    reopened.listener.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn stdio_tool_errors_remain_tool_errors_and_destination_cannot_be_spoofed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let fixture = Fixture::new(&directory.path().join("host.sqlite3")).await?;
    let client = fixture.connect().await?;
    let (request, _lease) = fixture.request("botus_apply", json!({"action":"create","title":"invalid","request_id":"invalid-batch","operations":[{"op":"imaginary"}]}))?;
    let result = client.call_tool(request).await?;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(body(&result)?["error"]["code"], "invalid_input");
    let args = serde_json::value::to_raw_value(&json!({"action":"identity"}))?;
    let mut spoof = fixture.config.clone();
    let McpTransportConfig::Stdio(config) = &mut spoof.transport else {
        return Err("Expected stdio".into());
    };
    config.command = "same-name-different-binary".into();
    assert!(
        fixture
            .provider
            .prepare(&spoof, "connection", "botus_read", &args, &fixture.context)?
            .is_none()
    );
    assert!(
        fixture
            .provider
            .prepare(
                &fixture.config,
                "connection",
                "botus_read",
                &args,
                &ToolDispatchContext::default()
            )
            .is_err()
    );
    let (mut request, _lease) = fixture.request("botus_read", json!({"action":"identity"}))?;
    request.arguments = json!({"action":"list"}).as_object().cloned();
    assert!(client.call_tool(request).await.is_err());
    client.cancel().await?;
    fixture.listener.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cancellation_and_dropped_native_lease_reach_the_host() -> TestResult {
    let directory = tempfile::tempdir()?;
    let fixture = Fixture::new(&directory.path().join("host.sqlite3")).await?;
    let client = fixture.connect().await?;
    fixture.authority.blocking.store(true, Ordering::SeqCst);
    let (request, lease) = fixture.request("botus_read", json!({"action":"identity"}))?;
    let handle = client
        .send_request_with_option(
            ClientRequest::CallToolRequest(CallToolRequest::new(request)),
            PeerRequestOptions::with_timeout(Duration::from_secs(5)),
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(5), fixture.authority.entered.notified()).await?;
    drop(lease);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), handle.await_response())
            .await?
            .is_err()
    );
    assert_eq!(fixture.authority.dropped.load(Ordering::SeqCst), 1);
    let (request, _lease) = fixture.request("botus_read", json!({"action":"identity"}))?;
    let handle = client
        .send_request_with_option(
            ClientRequest::CallToolRequest(CallToolRequest::new(request)),
            PeerRequestOptions::with_timeout(Duration::from_secs(5)),
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(5), fixture.authority.entered.notified()).await?;
    handle
        .cancel(Some("test native cancellation".into()))
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.authority.dropped.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    client.cancel().await?;
    fixture.listener.shutdown().await?;
    Ok(())
}

#[test]
fn launcher_refuses_missing_host_settings_before_protocol_start() -> TestResult {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_botus-1-2-3"))
        .arg("mcp")
        .env_remove("BOTUS_MCP_ENDPOINT")
        .env_remove("BOTUS_MCP_RELAY_TOKEN")
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BOTUS_MCP_ENDPOINT"));
    Ok(())
}

#[tokio::test]
async fn stdio_rejects_an_oversized_unterminated_frame() -> TestResult {
    use tokio::io::AsyncWriteExt;
    let directory = tempfile::tempdir()?;
    let fixture = Fixture::new(&directory.path().join("host.sqlite3")).await?;
    let McpTransportConfig::Stdio(config) = &fixture.config.transport else {
        return Err("Expected stdio".into());
    };
    let mut child = tokio::process::Command::new(&config.command)
        .args(&config.args)
        .envs(&config.env)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("Expected child stdin")?;
    let chunk = [b'x'; 16 * 1024];
    tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..520 {
            if input.write_all(&chunk).await.is_err() {
                break;
            }
        }
    })
    .await?;
    // Keep stdin open: an unbounded read_until would wait indefinitely for
    // EOF/newline and must fail this regression rather than pass at EOF.
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await??;
    drop(input);
    assert!(!status.success());
    fixture.listener.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn mcp_envelope_budget_rejects_before_mutation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let limit = botus_1_2_3::mcp::MIN_MCP_RESPONSE_BYTES;
    let fixture = Fixture::with_limits(
        &directory.path().join("host.sqlite3"),
        botus_1_2_3::Limits {
            max_response_bytes: limit,
            ..Default::default()
        },
    )
    .await?;
    let client = fixture.connect().await?;
    let (request, _lease) = fixture.request("botus_apply", json!({"action":"create","request_id":"escaped-selection","title":"Boundary","operations":[{"op":"put","at":"A1","values":[["\"\\".repeat(8000)]]}],"select":{"range":"A1"}}))?;
    let result = client.call_tool(request).await?;
    assert_eq!(result.is_error, Some(true));
    assert!(serde_json::to_vec(&result)?.len() <= limit);
    let (request, _lease) = fixture.request("botus_read", json!({"action":"list"}))?;
    let result = body(&client.call_tool(request).await?)?;
    assert_eq!(result["documents"], json!([]));
    client.cancel().await?;
    fixture.listener.shutdown().await?;
    let invalid = Fixture::with_limits(
        &directory.path().join("too-small.sqlite3"),
        botus_1_2_3::Limits {
            max_response_bytes: limit - 1,
            ..Default::default()
        },
    )
    .await;
    assert!(invalid.is_err());
    Ok(())
}

#[derive(Clone, Default)]
struct TraceBuffer(Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for TraceBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("trace buffer poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn endpoint_trace_logs_never_render_live_capabilities_or_relay_credentials() -> TestResult {
    let buffer = TraceBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let _tracing = tracing::subscriber::set_default(subscriber);
    let directory = tempfile::tempdir()?;
    let fixture = Fixture::new(&directory.path().join("host.sqlite3")).await?;
    let McpTransportConfig::Stdio(config) = &fixture.config.transport else {
        return Err("Expected stdio".into());
    };
    let endpoint = config
        .env
        .get("BOTUS_MCP_ENDPOINT")
        .ok_or("Missing endpoint")?;
    let credential = config
        .env
        .get("BOTUS_MCP_RELAY_TOKEN")
        .ok_or("Missing credential")?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let post = || {
        http.post(endpoint)
            .bearer_auth(credential)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
    };
    let response = post().body(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"trace-probe","version":"1"}}}).to_string()).send().await?;
    assert!(response.status().is_success());
    let session = response
        .headers()
        .get("mcp-session-id")
        .ok_or("Missing MCP session")?
        .to_str()?
        .to_owned();
    let _initialize = response.text().await?;
    let response = post()
        .header("mcp-session-id", &session)
        .body(json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string())
        .send()
        .await?;
    assert!(response.status().is_success());
    let (request, prepared) = fixture.request("botus_read", json!({"action":"identity"}))?;
    let token = prepared.metadata[botus_1_2_3::mcp::CALL_META_KEY]["token"]
        .as_str()
        .ok_or("Missing call token")?
        .to_owned();
    let response = post()
        .header("mcp-session-id", &session)
        .body(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":request}).to_string())
        .send()
        .await?;
    assert!(response.status().is_success());
    assert!(response.text().await?.contains("same-durable-agent"));
    // The malformed parser path must not include raw JSON in diagnostics.
    let response = post()
        .header("mcp-session-id", &session)
        .body(format!("{{\"token\":\"{token}\" broken JSON"))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let logs = String::from_utf8(
        buffer
            .0
            .lock()
            .map_err(|_| std::io::Error::other("trace buffer poisoned"))?
            .clone(),
    )?;
    assert!(logs.contains("received request"));
    assert!(!logs.contains(&token));
    assert!(!logs.contains(credential));
    fixture.listener.shutdown().await?;
    Ok(())
}
