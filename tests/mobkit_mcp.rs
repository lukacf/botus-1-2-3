#![cfg(feature = "mcp-host-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Full host acceptance: model loop -> native router -> stdio relay -> HTTP
//! MCP -> MobKit's current identity and durable document authority. No fixture
//! caller resolver, caller token, or direct document mutation is used here.

use async_trait::async_trait;
use botus_1_2_3::{
    mcp::{BotusMcpContextProvider, BotusMcpHost, BotusMcpListener, ENDPOINT_ENV, RELAY_TOKEN_ENV},
    native::{BotusToolFactory, NAMESPACE},
};
use futures::StreamExt;
use meerkat_client::{LlmClient, LlmDoneOutcome, LlmError, LlmEvent, LlmRequest};
use meerkat_core::{
    McpServerConfig, Message, SessionId, ToolDispatchContext, ToolSourceKind,
    mcp_config::McpTransportConfig,
    types::{ContentInput, HandlingMode, StopReason, ToolCallView, ToolResult},
};
use meerkat_mcp::{McpCallContext, McpCallContextError, McpCallContextProvider, McpCallTarget};
use meerkat_mob::{
    AgentIdentity, MemberCreationProvenance, MobDefinition, MobHandle, ProfileBinding, ProfileName,
    SpawnMemberSpec,
};
use meerkat_mobkit::{
    MobTerminalShutdownOutcome, UnifiedRuntime, UnifiedRuntimeShutdownReport,
    extension_state::{
        ChildMcpAvailability, HostMcpDocumentBinding,
        documents::{Audience, DocumentAccess, Grant, Owner, Principal, Reach, Role},
    },
    mob_composition_manifest::persistent_mob_storage,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const TURN_TIMEOUT: Duration = Duration::from_secs(45);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(20);
const CATALOG_BACKOFF: Duration = Duration::from_millis(250);
static CALL_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Default)]
enum CatalogStep {
    #[default]
    Start,
    Search(String),
    Load(String),
    Ready,
}

#[derive(Clone)]
struct Plan {
    marker: String,
    tool: String,
    arguments: Value,
    expects_botus: bool,
    result: Option<ToolResult>,
    catalog_step: CatalogStep,
    catalog_started: Option<Instant>,
    catalog_calls: usize,
    absent_proven: bool,
    planned_emitted: bool,
}

/// The fake component is only the language model. Every emitted tool call is
/// admitted and executed by the same runtime path as a provider-generated call.
#[derive(Default)]
struct ScriptClient {
    plans: Mutex<BTreeMap<String, Plan>>,
    completed: tokio::sync::Notify,
}

impl ScriptClient {
    fn plan(&self, tool: &str, arguments: Value, expects_botus: bool) -> String {
        // A fresh client after restart must not reuse a tool-use id already
        // present in the persisted model history.
        let id = format!(
            "botus-acceptance-{}",
            CALL_SEQUENCE.fetch_add(1, Ordering::SeqCst)
        );
        let marker = format!("BOTUS_ACCEPTANCE[{id}]END");
        self.plans.lock().unwrap().insert(
            id.clone(),
            Plan {
                marker,
                tool: tool.into(),
                arguments,
                expects_botus,
                result: None,
                catalog_step: CatalogStep::Start,
                catalog_started: None,
                catalog_calls: 0,
                absent_proven: false,
                planned_emitted: false,
            },
        );
        id
    }

    fn prompt(&self, id: &str) -> String {
        self.plans.lock().unwrap()[id].marker.clone()
    }

    async fn result(&self, id: &str) -> ToolResult {
        tokio::time::timeout(TURN_TIMEOUT, async {
            loop {
                let notified = self.completed.notified();
                if let Some(result) = self.plans.lock().unwrap()[id].result.clone() {
                    return result;
                }
                notified.await;
            }
        })
        .await
        .expect("scripted tool call did not return to the model")
    }

    fn next_call(&self, id: &str, request: &LlmRequest) -> (LlmEvent, bool) {
        let mut plans = self.plans.lock().unwrap();
        let plan = plans.get_mut(id).unwrap();
        let started = *plan.catalog_started.get_or_insert_with(Instant::now);
        assert!(
            started.elapsed() < CATALOG_TIMEOUT,
            "Botus catalog readiness timed out"
        );
        let has_search = request
            .tools
            .iter()
            .any(|tool| tool.name == "tool_catalog_search");
        let mut search = false;
        let mut load = false;
        let mut backoff = false;
        match plan.catalog_step.clone() {
            CatalogStep::Start => {
                if has_search {
                    search = true;
                } else {
                    // Without any deferred/dynamic source the native factory
                    // does not compose catalog controls. The HostOnly test also
                    // verifies the real implicit child's public profile below.
                    assert!(
                        !plan.expects_botus,
                        "MCP source has no catalog control plane"
                    );
                    assert!(
                        !request
                            .tools
                            .iter()
                            .any(|tool| tool.name == "tool_catalog_load")
                    );
                    plan.absent_proven = true;
                    plan.catalog_step = CatalogStep::Ready;
                }
            }
            CatalogStep::Search(call_id) => {
                let response = success(tool_result(request, &call_id).expect("search result"));
                assert_eq!(response["catalog_exact"], true);
                let results = response["results"].as_array().unwrap();
                let botus = results
                    .iter()
                    .filter(|entry| {
                        matches!(entry["name"].as_str(), Some("botus_read" | "botus_apply"))
                    })
                    .collect::<Vec<_>>();
                let pending = response["pending_sources"].as_array();
                let has_pending = pending.is_some_and(|sources| !sources.is_empty());
                let both_loadable = botus.len() == 2
                    && botus.iter().all(|entry| {
                        matches!(
                            entry["visibility_status"].as_str(),
                            Some("deferred" | "loaded")
                        )
                    });
                if plan.expects_botus && both_loadable {
                    load = true;
                } else if has_pending {
                    if plan.expects_botus {
                        assert!(
                            pending.unwrap().iter().any(|source| source == NAMESPACE),
                            "Botus is absent and its source is not pending"
                        );
                        assert!(
                            !botus
                                .iter()
                                .any(|entry| entry["visibility_status"] == "blocked_by_filter"),
                            "Botus is blocked by the current inherited tool filter"
                        );
                    } else {
                        assert!(botus.is_empty(), "HostOnly leaked a Botus catalog entry");
                    }
                    if results.is_empty() {
                        assert_eq!(response["empty_result_status"], "pending_source");
                    }
                    search = true;
                    backoff = true;
                } else {
                    assert!(
                        !plan.expects_botus,
                        "exact completed catalog is missing Botus"
                    );
                    assert!(botus.is_empty(), "HostOnly leaked a Botus catalog entry");
                    plan.absent_proven = true;
                    plan.catalog_step = CatalogStep::Ready;
                }
            }
            CatalogStep::Load(call_id) => {
                let response = success(tool_result(request, &call_id).expect("load result"));
                assert_eq!(response["catalog_exact"], true);
                let resolutions = response["resolutions"].as_array().unwrap();
                assert_eq!(resolutions.len(), 2);
                for name in ["botus_read", "botus_apply"] {
                    let resolution = resolutions
                        .iter()
                        .find(|entry| entry["name"] == name)
                        .unwrap();
                    assert_eq!(resolution["accepted"], true, "load failed: {resolution}");
                    assert!(resolution.get("rejected_reason").is_none());
                }
                plan.catalog_step = CatalogStep::Ready;
            }
            CatalogStep::Ready => {}
        }
        if search || load {
            plan.catalog_calls += 1;
            assert!(
                plan.catalog_calls <= 96,
                "catalog search exceeded its call bound"
            );
            let call_id = format!("{id}:catalog:{}", plan.catalog_calls);
            let (name, args) = if search {
                assert!(has_search, "catalog search disappeared while pending");
                plan.catalog_step = CatalogStep::Search(call_id.clone());
                ("tool_catalog_search", json!({"query":"botus","limit":10}))
            } else {
                assert!(
                    request
                        .tools
                        .iter()
                        .any(|tool| tool.name == "tool_catalog_load")
                );
                plan.catalog_step = CatalogStep::Load(call_id.clone());
                (
                    "tool_catalog_load",
                    json!({"names":["botus_read","botus_apply"]}),
                )
            };
            return (
                LlmEvent::ToolCallComplete {
                    id: call_id,
                    name: name.into(),
                    args,
                    meta: None,
                },
                backoff,
            );
        }
        assert!(matches!(plan.catalog_step, CatalogStep::Ready));
        let mut botus = request
            .tools
            .iter()
            .filter(|tool| matches!(tool.name.as_str(), "botus_read" | "botus_apply"))
            .collect::<Vec<_>>();
        botus.sort_by(|a, b| a.name.cmp(&b.name));
        if plan.expects_botus {
            assert_eq!(botus.len(), 2, "Botus must expose exactly two loaded tools");
            assert_eq!(botus[0].name, "botus_apply");
            assert_eq!(botus[1].name, "botus_read");
            for tool in botus {
                let provenance = tool.provenance.as_ref().unwrap();
                assert_eq!(provenance.kind, ToolSourceKind::Mcp);
                assert_eq!(provenance.source_id.as_str(), NAMESPACE);
            }
        } else {
            assert!(plan.absent_proven);
            assert!(
                botus.is_empty(),
                "HostOnly must not be supplied to delegates"
            );
        }
        plan.planned_emitted = true;
        (
            LlmEvent::ToolCallComplete {
                id: id.into(),
                name: plan.tool.clone(),
                args: plan.arguments.clone(),
                meta: None,
            },
            false,
        )
    }
}

fn tool_result(request: &LlmRequest, id: &str) -> Option<ToolResult> {
    request.messages.iter().rev().find_map(|message| {
        let Message::ToolResults { results, .. } = message else {
            return None;
        };
        results
            .iter()
            .find(|result| result.tool_use_id == id)
            .cloned()
    })
}

#[async_trait]
impl LlmClient for ScriptClient {
    fn project_replay_messages(&self, messages: &[Message]) -> Result<Vec<Message>, LlmError> {
        Ok(messages.to_vec())
    }

    fn provider(&self) -> meerkat_core::Provider {
        meerkat_core::Provider::OpenAI
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        Ok(())
    }

    fn stream<'a>(
        &'a self,
        request: &'a LlmRequest,
    ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
        let selected = {
            let plans = self.plans.lock().unwrap();
            request.messages.iter().rev().find_map(|message| {
                let Message::User(user) = message else {
                    return None;
                };
                let text = user.text_content();
                plans
                    .iter()
                    .find(|(_, plan)| text.contains(&plan.marker))
                    .map(|(id, plan)| (id.clone(), plan.clone()))
            })
        };
        let mut events = Vec::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut backoff = false;
        if let Some((id, _)) = selected {
            let result = tool_result(request, &id);
            if let Some(result) = result {
                self.plans.lock().unwrap().get_mut(&id).unwrap().result = Some(result);
                self.completed.notify_one();
            } else {
                let (call, pending) = self.next_call(&id, request);
                events.push(Ok(call));
                backoff = pending;
                stop_reason = StopReason::ToolUse;
            }
        }
        if stop_reason == StopReason::EndTurn {
            events.push(Ok(LlmEvent::TextDelta {
                delta: "done".into(),
                meta: None,
            }));
        }
        events.push(Ok(LlmEvent::UsageUpdate {
            usage: meerkat_core::TurnUsage::host_declared(
                self.provider(),
                &request.model,
                meerkat_core::Usage::default(),
            ),
        }));
        events.push(Ok(LlmEvent::Done {
            outcome: LlmDoneOutcome::Success { stop_reason },
        }));
        Box::pin(
            futures::stream::once(async move {
                // Yield only after an actual catalog response says a source is
                // still pending. The next real search observes fresh native state.
                if backoff {
                    tokio::time::sleep(CATALOG_BACKOFF).await;
                }
                futures::stream::iter(events)
            })
            .flatten(),
        )
    }
}

#[derive(Clone)]
struct PreparedObservation {
    call_id: String,
    operation: String,
    session: SessionId,
}

struct ObservedProvider {
    inner: BotusMcpContextProvider,
    calls: Mutex<Vec<PreparedObservation>>,
}

#[async_trait]
impl McpCallContextProvider for ObservedProvider {
    async fn prepare(
        &self,
        target: McpCallTarget<'_>,
        call: ToolCallView<'_>,
        context: &ToolDispatchContext,
    ) -> Result<Option<McpCallContext>, McpCallContextError> {
        self.calls.lock().unwrap().push(PreparedObservation {
            call_id: call.id.into(),
            operation: target.raw_operation.into(),
            session: context.origin_session_id().cloned().expect("native origin"),
        });
        // Preserve the complete native target, including its origin projection.
        // The observer cannot mint caller identity or a capability token.
        McpCallContextProvider::prepare(&self.inner, target, call, context).await
    }
    // Deliberately retain the default Unknown classification: the validated
    // MobKit document requirements, not this wrapper, classify the tools.
}

struct Host {
    runtime: UnifiedRuntime,
    listener: BotusMcpListener,
    client: Arc<ScriptClient>,
    provider: Arc<ObservedProvider>,
    public: McpServerConfig,
    runtime_config: McpServerConfig,
}

fn definition(id: &str, descriptor: &McpServerConfig) -> MobDefinition {
    let mut definition = MobDefinition::from_toml(
        &r#"
[mob]
id = "MOB_ID"
[profiles.worker]
model = "gpt-5.5"
runtime_mode = "turn_driven"
external_addressable = true
[profiles.worker.tools]
comms = true
mob = true
[profiles.reader]
model = "gpt-5.5"
runtime_mode = "turn_driven"
external_addressable = true
[profiles.reader.tools]
comms = true
read_only = true
"#
        .replace("MOB_ID", id),
    )
    .unwrap();
    for binding in definition.profiles.values_mut() {
        let ProfileBinding::Inline(profile) = binding else {
            panic!("test profiles are inline")
        };
        profile.tools.mcp_servers.push(descriptor.clone());
    }
    definition
}

impl Host {
    async fn start(path: &Path, mob_id: &str, availability: ChildMcpAvailability) -> Self {
        let host = Arc::new(BotusMcpHost::default());
        let listener = host.start().await.unwrap();
        let runtime_config = listener.relay_config(env!("CARGO_BIN_EXE_botus-1-2-3"));
        let provider = Arc::new(ObservedProvider {
            inner: listener
                .call_context_provider(runtime_config.clone())
                .unwrap(),
            calls: Mutex::new(Vec::new()),
        });
        let mut public = runtime_config.clone();
        let McpTransportConfig::Stdio(stdio) = &mut public.transport else {
            panic!("relay is stdio")
        };
        for value in stdio.env.values_mut() {
            value.clear();
        }
        // This is the typed host registration, with a transparent observer
        // around the real provider. The factory is never installed natively.
        let binding = HostMcpDocumentBinding::new(
            public,
            runtime_config.clone(),
            BotusToolFactory::requirements(),
            host,
            provider.clone(),
            availability,
        )
        .unwrap();
        let public = binding.public_config().clone();
        let client = Arc::new(ScriptClient::default());
        let (storage, provenance) = persistent_mob_storage(path.join("mob.sqlite3")).unwrap();
        let mut agent_config = meerkat_core::Config::default();
        agent_config.budget.max_tool_calls = Some(256);
        agent_config.budget.max_tokens = Some(200_000);
        let runtime = Box::pin(
            UnifiedRuntime::builder()
                .definition(definition(mob_id, &public))
                .persistent_state(path)
                .mob_storage(storage, provenance)
                .meerkat_config(agent_config)
                .default_llm_client(client.clone())
                .register_mcp_tool_bundle_factory(binding)
                .build(),
        )
        .await
        .unwrap();
        Self {
            runtime,
            listener,
            client,
            provider,
            public,
            runtime_config,
        }
    }

    async fn spawn(&self, profile: &str, id: &str) {
        self.runtime
            .spawn(SpawnMemberSpec::host_root(profile, AgentIdentity::from(id)))
            .await
            .unwrap();
    }

    async fn call(&self, member: &str, tool: &str, arguments: Value) -> ToolResult {
        self.call_on(&self.runtime.mob_handle(), member, tool, arguments, true)
            .await
    }

    async fn call_on(
        &self,
        handle: &MobHandle,
        member: &str,
        tool: &str,
        arguments: Value,
        expects_botus: bool,
    ) -> ToolResult {
        let id = self.client.plan(tool, arguments, expects_botus);
        let member = handle.member(&AgentIdentity::from(member)).await.unwrap();
        let turn = member
            .start_turn(
                ContentInput::Text(self.client.prompt(&id)),
                HandlingMode::Queue,
                meerkat_mob::MemberTurnOptions::new(),
                None,
            )
            .await
            .unwrap();
        tokio::time::timeout(TURN_TIMEOUT, turn.wait())
            .await
            .unwrap()
            .unwrap();
        self.client.result(&id).await
    }

    async fn identity(&self, member: &str) -> Principal {
        serde_json::from_value(
            success(
                self.call(member, "botus_read", json!({"action":"identity"}))
                    .await,
            )["caller_principal"]
                .clone(),
        )
        .unwrap()
    }

    async fn session(&self, member: &str) -> SessionId {
        self.runtime
            .mob_handle()
            .get_member(&AgentIdentity::from(member))
            .await
            .unwrap()
            .unwrap()
            .bridge_session_id()
            .unwrap()
            .clone()
    }

    async fn share(
        &self,
        id: &Value,
        revision: &Value,
        key: &str,
        access: DocumentAccess,
    ) -> Value {
        success(self.call("owner", "botus_apply", json!({
            "action":"share", "request_id":key, "id":id, "revision":revision, "access":access,
        })).await)["receipt"]["revision"].clone()
    }

    async fn stop(self) {
        let shutdown = self.runtime.shutdown().await;
        assert_clean_shutdown(&shutdown);
        drop(self.runtime);
        self.listener.shutdown().await.unwrap();
    }
}

fn assert_clean_shutdown(shutdown: &UnifiedRuntimeShutdownReport) {
    assert!(
        shutdown.cleanup_completed(),
        "incomplete shutdown: {shutdown:?}"
    );
    // cleanup_completed deliberately excludes terminal supervisor teardown.
    // Same-process reopen also requires that exact route to be released.
    match &shutdown.mob_terminal_shutdown {
        MobTerminalShutdownOutcome::Completed(report) => {
            assert!(
                report.is_clean(),
                "incomplete terminal shutdown: {report:?}"
            );
        }
        MobTerminalShutdownOutcome::AlreadyShutDown => {}
        other => panic!("terminal shutdown did not complete: {other:?}"),
    }
}

fn success(result: ToolResult) -> Value {
    assert!(!result.is_error, "tool failed: {}", result.text_content());
    let value: Value = serde_json::from_str(&result.text_content()).unwrap();
    assert!(value.get("error").is_none(), "tool refused: {value}");
    value
}

fn unavailable(result: ToolResult) {
    assert!(result.is_error, "a denied tool must retain isError");
    let value: Value = serde_json::from_str(&result.text_content()).unwrap();
    assert_eq!(value["error"]["code"], "unavailable");
}

fn create(request: &str) -> Value {
    json!({"action":"create", "request_id":request, "title":"The recalculation is coming from inside the mob",
        "operations":[{"op":"put","at":"A1","values":[[7,{"formula":"=A1*3"}]]}]})
}

fn edit(id: &Value, revision: &Value, key: &str, value: u64) -> Value {
    json!({"action":"edit", "request_id":key, "id":id, "revision":revision,
        "operations":[{"op":"put","at":"A1","values":[[value]]}]})
}

fn describe(id: &Value) -> Value {
    json!({"action":"describe","id":id})
}

fn private() -> DocumentAccess {
    DocumentAccess {
        owner_reach: Reach::SelfOnly,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_mcp_identity_fork_off_child_acl_and_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::start(
        dir.path(),
        "botus-mcp-lineage",
        ChildMcpAvailability::HostOnly,
    )
    .await;
    host.spawn("worker", "owner").await;
    let owner = host.identity("owner").await;
    let owner_session = host.session("owner").await;
    let created = success(host.call("owner", "botus_apply", create("initial")).await);
    let id = created["receipt"]["document_id"].clone();
    let mut revision = created["receipt"]["revision"].clone();
    let initial = success(host.call("owner", "botus_read", describe(&id)).await);
    assert_eq!(
        initial["owner"],
        serde_json::to_value(Owner::Agent(owner.clone())).unwrap()
    );
    assert_eq!(
        initial["access"],
        serde_json::to_value(DocumentAccess::default()).unwrap()
    );

    let fork_read = host.client.plan("botus_read", describe(&id), true);
    success(
        host.call(
            "owner",
            "fork_off",
            json!({
                "member_id":"fork", "task":host.client.prompt(&fork_read),
                "idle_retire_secs":3600, "max_text_bytes":1024,
            }),
        )
        .await,
    );
    success(host.client.result(&fork_read).await);
    // This tracked turn also waits for the initial fork job to finish.
    let fork = host.identity("fork").await;
    let fork_session = host.session("fork").await;
    assert_ne!(owner, fork);
    assert_ne!(owner_session, fork_session);
    let creation = host
        .runtime
        .mob_handle()
        .member_creation_for_session(&fork_session)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        creation.creation.provenance,
        MemberCreationProvenance::Fork { .. }
    ));
    assert_eq!(
        creation.fork_source.unwrap().source_session_id,
        owner_session
    );
    let observed = host
        .provider
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|call| call.call_id == fork_read)
        .cloned()
        .unwrap();
    assert_eq!(observed.session, fork_session);
    assert_eq!(observed.operation, "botus_read");

    success(
        host.call(
            "owner",
            "spawn_member",
            json!({
                "profile":"worker", "member_id":"child",
            }),
        )
        .await,
    );
    assert_ne!(owner, host.identity("child").await);
    let child_session = host.session("child").await;
    let child_creation = host
        .runtime
        .mob_handle()
        .member_creation_for_session(&child_session)
        .await
        .unwrap()
        .unwrap();
    let MemberCreationProvenance::Spawn { source } = child_creation.creation.provenance else {
        panic!("spawn_member must capture its native caller source")
    };
    assert_eq!(source.session_id, owner_session);
    unavailable(host.call("child", "botus_read", describe(&id)).await);
    host.spawn("worker", "peer").await;
    unavailable(host.call("peer", "botus_read", describe(&id)).await);

    revision = success(
        host.call("fork", "botus_apply", edit(&id, &revision, "fork-edit", 9))
            .await,
    )["receipt"]["revision"]
        .clone();
    unavailable(
        host.call(
            "fork",
            "botus_apply",
            json!({
                "action":"share", "request_id":"fork-cannot-share", "id":id,
                "revision":revision, "access":private(),
            }),
        )
        .await,
    );
    revision = host
        .share(
            &id,
            &revision,
            "share-descendants",
            DocumentAccess {
                owner_reach: Reach::Descendants,
                ..Default::default()
            },
        )
        .await;
    revision = success(
        host.call(
            "child",
            "botus_apply",
            edit(&id, &revision, "child-edit", 11),
        )
        .await,
    )["receipt"]["revision"]
        .clone();

    host.spawn("reader", "reader").await;
    let reader = host.identity("reader").await;
    revision = host
        .share(
            &id,
            &revision,
            "reader-editor-grant",
            DocumentAccess {
                owner_reach: Reach::Descendants,
                grants: vec![Grant {
                    audience: Audience::Agent {
                        principal: reader,
                        reach: Reach::SelfOnly,
                    },
                    role: Role::Editor,
                }],
                ..Default::default()
            },
        )
        .await;
    let read = success(
        host.call(
            "reader",
            "botus_read",
            json!({
                "action":"read", "id":id, "query":{"range":"A1:B1"},
            }),
        )
        .await,
    );
    assert_eq!(read["selected"]["rows"], json!([[11.0, 33.0]]));
    let prepared_before = host.provider.calls.lock().unwrap().len();
    let denied = host
        .call(
            "reader",
            "botus_apply",
            edit(&id, &revision, "read-only-denied", 99),
        )
        .await;
    assert!(
        denied.is_error,
        "ReadOnly must refuse the advertised write tool"
    );
    assert_eq!(
        host.provider.calls.lock().unwrap().len(),
        prepared_before,
        "the native policy must refuse before creating a call capability"
    );
    assert_eq!(
        success(host.call("owner", "botus_read", describe(&id)).await)["revision"],
        revision
    );

    revision = host.share(&id, &revision, "revoke", private()).await;
    for member in ["fork", "child", "reader", "peer"] {
        unavailable(host.call(member, "botus_read", describe(&id)).await);
    }
    assert_eq!(
        success(host.call("owner", "botus_read", describe(&id)).await)["revision"],
        revision
    );
    host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_mcp_child_supply_and_copied_host_only_descriptor() {
    for availability in [
        ChildMcpAvailability::HostOnly,
        ChildMcpAvailability::ChildAvailable,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start(dir.path(), "botus-mcp-child-supply", availability).await;
        host.spawn("worker", "owner").await;
        let owner = host.identity("owner").await;
        let available = availability == ChildMcpAvailability::ChildAvailable;
        let delegated = host
            .client
            .plan("botus_read", json!({"action":"identity"}), available);
        let delegation = host
            .call(
                "owner",
                "delegate",
                json!({
                    "member_id":"helper", "task":host.client.prompt(&delegated),
                    "result_label":"botus-helper-result", "max_text_bytes":1024,
                }),
            )
            .await;
        if available {
            success(delegation);
            let helper_result = host.client.result(&delegated).await;
            let helper: Principal =
                serde_json::from_value(success(helper_result)["caller_principal"].clone()).unwrap();
            let Principal::Worker {
                mob_id, member_id, ..
            } = &helper
            else {
                panic!("delegate has Worker identity")
            };
            assert_ne!(mob_id, host.runtime.mob_handle().mob_id().as_str());
            assert_eq!(member_id, "helper");
            assert_ne!(helper, owner);
        } else {
            assert!(
                delegation.is_error,
                "HostOnly must not be supplied to implicit delegates"
            );
            assert!(
                delegation
                    .text_content()
                    .contains("Tool not found: botus_read"),
                "delegate failed for a different reason: {}",
                delegation.text_content()
            );
            let plans = host.client.plans.lock().unwrap();
            let helper = &plans[&delegated];
            assert!(helper.absent_proven && helper.planned_emitted);
        }

        // The public mob_create input already excludes MCP launch configs.
        // Exercise defense in depth by creating an actual native child that
        // nevertheless contains a copied public descriptor. Its owner bridge,
        // not the profile name or descriptor presence, determines child scope.
        let state = host.runtime.mob_runtime().agent_mob_mcp_state().unwrap();
        let implicit_id = state
            .find_implicit_mob_for_bridge_session(&host.session("owner").await.to_string())
            .await
            .expect("delegate must have created its native implicit mob");
        let implicit = state.handle_for(&implicit_id).await.unwrap();
        for binding in implicit.definition().profiles.values() {
            let ProfileBinding::Inline(profile) = binding else {
                panic!("inline delegate profile")
            };
            if available {
                assert_eq!(profile.tools.mcp_servers, vec![host.public.clone()]);
            } else {
                assert!(
                    profile.tools.mcp_servers.is_empty(),
                    "HostOnly descriptor was persisted in implicit child"
                );
            }
        }
        drop(implicit);
        let child_id = state
            .mob_create_definition_with_owner_bridge_session(
                definition("copied-descriptor-child", &host.public),
                host.session("owner").await,
                true,
                false,
            )
            .await
            .unwrap();
        let child = state.handle_for(&child_id).await.unwrap();
        assert!(
            child
                .owner_bridge_session_lifecycle_authority()
                .unwrap()
                .destroy_on_owner_archive
        );
        child
            .spawn_spec(SpawnMemberSpec::host_root(
                "worker",
                AgentIdentity::from("copied"),
            ))
            .await
            .unwrap();
        let copied = host
            .call_on(
                &child,
                "copied",
                "botus_read",
                json!({"action":"identity"}),
                true,
            )
            .await;
        if available {
            let caller: Principal =
                serde_json::from_value(success(copied)["caller_principal"].clone()).unwrap();
            assert_ne!(caller, owner);
        } else {
            unavailable(copied);
        }
        let ProfileBinding::Inline(profile) =
            &child.definition().profiles[&ProfileName::from("worker")]
        else {
            panic!("inline")
        };
        assert_eq!(profile.tools.mcp_servers, vec![host.public.clone()]);
        drop(child);
        // Stop alone keeps child supervisor routes alive. Canonical owner
        // cleanup destroys both the explicit and implicit child before the
        // next availability case reuses their names.
        state
            .destroy_bridge_session_mobs(&host.session("owner").await.to_string())
            .await
            .unwrap();
        drop(state);
        host.stop().await;
    }
}

fn assert_credentials_absent(path: &Path, credential: &str) {
    assert!(!credential.is_empty());
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_credentials_absent(&path, credential);
        } else if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes
                    .windows(credential.len())
                    .any(|window| window == credential.as_bytes()),
                "host relay credential persisted in {}",
                path.display()
            );
        }
    }
}

fn relay_setting(config: &McpServerConfig, key: &str) -> String {
    let McpTransportConfig::Stdio(stdio) = &config.transport else {
        panic!("stdio")
    };
    stdio.env[key].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn native_mcp_restart_rebinds_transport_preserves_acl_and_refuses_withdrawal() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::start(
        dir.path(),
        "botus-mcp-restart",
        ChildMcpAvailability::ChildAvailable,
    )
    .await;
    host.spawn("worker", "owner").await;
    let owner = host.identity("owner").await;
    let owner_session = host.session("owner").await;
    let created = success(
        host.call("owner", "botus_apply", create("durable-create"))
            .await,
    );
    let id = created["receipt"]["document_id"].clone();
    let revision = created["receipt"]["revision"].clone();
    let fork_read = host.client.plan("botus_read", describe(&id), true);
    success(
        host.call(
            "owner",
            "fork_off",
            json!({
                "member_id":"fork", "task":host.client.prompt(&fork_read),
                "idle_retire_secs":3600, "max_text_bytes":1024,
            }),
        )
        .await,
    );
    success(host.client.result(&fork_read).await);
    let fork = host.identity("fork").await;
    let fork_session = host.session("fork").await;
    let creation = host
        .runtime
        .mob_handle()
        .member_creation_for_session(&fork_session)
        .await
        .unwrap()
        .unwrap();
    let revoked_revision = host
        .share(&id, &revision, "durable-revoke", private())
        .await;
    unavailable(host.call("fork", "botus_read", describe(&id)).await);
    let public = host.public.clone();
    let first_runtime = host.runtime_config.clone();
    let first_secret = relay_setting(&first_runtime, RELAY_TOKEN_ENV);
    let first_endpoint = relay_setting(&first_runtime, ENDPOINT_ENV);
    host.stop().await;
    assert_credentials_absent(dir.path(), &first_secret);
    assert_credentials_absent(dir.path(), &first_endpoint);

    let host = Host::start(
        dir.path(),
        "botus-mcp-restart",
        ChildMcpAvailability::ChildAvailable,
    )
    .await;
    assert_eq!(
        host.public, public,
        "the persisted public selection is stable"
    );
    assert!(
        host.runtime_config != first_runtime,
        "a restart must create a new host transport"
    );
    assert!(
        relay_setting(&host.runtime_config, RELAY_TOKEN_ENV) != first_secret,
        "relay credentials must rotate"
    );
    assert_eq!(host.session("owner").await, owner_session);
    assert_eq!(host.session("fork").await, fork_session);
    assert_eq!(host.identity("owner").await, owner);
    assert_eq!(host.identity("fork").await, fork);
    assert_eq!(
        host.runtime
            .mob_handle()
            .member_creation_for_session(&fork_session)
            .await
            .unwrap()
            .unwrap(),
        creation
    );
    unavailable(host.call("fork", "botus_read", describe(&id)).await);
    let restored = success(host.call("owner", "botus_read", describe(&id)).await);
    assert_eq!(restored["revision"], revoked_revision);
    assert_eq!(restored["access"], serde_json::to_value(private()).unwrap());
    let revision = host
        .share(
            &id,
            &revoked_revision,
            "restored-fork-grant",
            DocumentAccess::default(),
        )
        .await;
    success(
        host.call(
            "fork",
            "botus_apply",
            edit(&id, &revision, "restored-fork-edit", 12),
        )
        .await,
    );
    let read = success(
        host.call(
            "owner",
            "botus_read",
            json!({
                "action":"read", "id":id, "query":{"range":"A1:B1"},
            }),
        )
        .await,
    );
    assert_eq!(read["selected"]["rows"], json!([[12.0, 36.0]]));
    let second_secret = relay_setting(&host.runtime_config, RELAY_TOKEN_ENV);
    let second_endpoint = relay_setting(&host.runtime_config, ENDPOINT_ENV);
    host.stop().await;
    assert_credentials_absent(dir.path(), &first_secret);
    assert_credentials_absent(dir.path(), &second_secret);
    assert_credentials_absent(dir.path(), &second_endpoint);

    // Reopen the same persisted mob and public selection without registering
    // the package. The missing binding must fail visibly, never revive with a
    // stale endpoint or a native dispatcher fallback.
    let client = Arc::new(ScriptClient::default());
    let (storage, provenance) = persistent_mob_storage(dir.path().join("mob.sqlite3")).unwrap();
    let reopened = Box::pin(
        UnifiedRuntime::builder()
            .definition(definition("botus-mcp-restart", &public))
            .persistent_state(dir.path())
            .mob_storage(storage, provenance)
            .default_llm_client(client.clone())
            .build(),
    )
    .await;
    let refusal = match reopened {
        Err(error) => error.to_string(),
        Ok(runtime) => {
            // A runtime may retain a failed restored member for repair. A new
            // member selecting the same durable profile must still refuse at
            // construction, before it could publish an unprotected tool.
            let error = runtime
                .spawn(SpawnMemberSpec::host_root(
                    "worker",
                    AgentIdentity::from("withdrawn-probe"),
                ))
                .await
                .expect_err("withdrawn package unexpectedly built an agent");
            let shutdown = runtime.shutdown().await;
            assert_clean_shutdown(&shutdown);
            error.to_string()
        }
    };
    assert!(
        refusal.contains("document MCP host binding is unavailable"),
        "unexpected refusal: {refusal}"
    );
}
