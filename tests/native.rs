#![cfg(feature = "mobkit")]
//! Adapter tests use the real SQLite backend and a deliberately explicit test
//! authority. Runtime lineage and full MobKit host restart need host tests too.

use async_trait::async_trait;
use botus_1_2_3::{
    Limits,
    native::{BotusToolFactory, MAX_TOOL_DEFINITION_BYTES, NAMESPACE, tool_definitions},
};
use meerkat_core::{
    AgentToolDispatcher, SessionId, ToolCallView, ToolDispatchContext, ToolError, ToolMutationClass,
};
use meerkat_mobkit::extension_state::{
    ToolBundleContext, ToolBundleFactory, ToolCallerResolver, documents::*,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TestAuthority {
    callers: BTreeMap<String, HostAccessContext>,
    calls: AtomicUsize,
}
#[async_trait]
impl ToolCallerResolver for TestAuthority {
    async fn resolve(
        &self,
        context: &ToolDispatchContext,
        requested_owner: Option<&Owner>,
    ) -> meerkat_mobkit::extension_state::documents::Result<HostAccessContext> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let session = context
            .origin_session_id()
            .ok_or(Error::AuthorityUnavailable)?;
        let caller = self
            .callers
            .get(&session.to_string())
            .cloned()
            .ok_or(Error::AuthorityUnavailable)?;
        if requested_owner.is_some_and(
            |owner| !matches!(owner, Owner::Agent(principal) if principal == caller.principal()),
        ) {
            return Err(Error::NotFound);
        }
        Ok(caller)
    }
}

fn actor(name: &str) -> TestResult<(ToolDispatchContext, HostAccessContext)> {
    let session = SessionId::new();
    Ok((
        ToolDispatchContext::default().with_runtime_identity(session, None),
        HostAccessContext::new(
            "beige-realm",
            Principal::Agent(name.into()),
            VerifiedLineage::root(),
            HostPolicy::ALLOW,
        )?,
    ))
}
fn authority(
    callers: &[(ToolDispatchContext, HostAccessContext)],
) -> TestResult<Arc<TestAuthority>> {
    let mut map = BTreeMap::new();
    for (context, caller) in callers {
        map.insert(
            context
                .origin_session_id()
                .ok_or("missing test session")?
                .to_string(),
            caller.clone(),
        );
    }
    Ok(Arc::new(TestAuthority {
        callers: map,
        calls: AtomicUsize::new(0),
    }))
}
fn tools(
    path: &Path,
    authority: Arc<TestAuthority>,
    limits: Limits,
) -> TestResult<Arc<dyn AgentToolDispatcher>> {
    let documents = DocumentService::new(
        "beige-realm",
        NAMESPACE,
        Arc::new(SqliteExtensionDocumentStore::open(path)?),
    )?;
    Ok(BotusToolFactory { limits }
        .build(ToolBundleContext {
            documents,
            caller_resolver: authority,
        })
        .map_err(std::io::Error::other)?)
}
async fn send(
    tools: &Arc<dyn AgentToolDispatcher>,
    caller: &ToolDispatchContext,
    name: &str,
    args: Value,
) -> TestResult<(bool, Value)> {
    let raw = serde_json::value::to_raw_value(&args)?;
    let outcome = tools
        .dispatch_with_context(
            ToolCallView {
                id: "test-call",
                name,
                args: &raw,
            },
            caller,
        )
        .await?;
    let encoded = serde_json::to_value(&outcome.result)?;
    let content = encoded["content"].as_str().ok_or("expected text result")?;
    Ok((outcome.result.is_error, serde_json::from_str(content)?))
}
async fn assert_authority_unavailable(
    tools: &Arc<dyn AgentToolDispatcher>,
    caller: &ToolDispatchContext,
    name: &str,
    args: Value,
) -> TestResult {
    let raw = serde_json::value::to_raw_value(&args)?;
    let result = tools
        .dispatch_with_context(
            ToolCallView {
                id: "unavailable-test-call",
                name,
                args: &raw,
            },
            caller,
        )
        .await;
    assert!(matches!(
        result,
        Err(ToolError::ExecutionFailed { message }) if message == "Caller authority unavailable"
    ));
    Ok(())
}
async fn create(
    tools: &Arc<dyn AgentToolDispatcher>,
    caller: &ToolDispatchContext,
) -> TestResult<Value> {
    let (error, result) = send(tools, caller, "botus_apply", json!({"action":"create","request_id":"new-floppy","title":"F9 retirement fund","operations":[{"op":"put","at":"A1","values":[[1,"Unicode: räksmörgås"]]}]})).await?;
    assert!(!error, "{result}");
    Ok(result["receipt"].clone())
}

#[tokio::test]
async fn schemas_are_compact_and_authority_is_never_an_argument() -> TestResult {
    assert!(serde_json::to_vec(&tool_definitions())?.len() <= MAX_TOOL_DEFINITION_BYTES);
    let temporary = tempfile::tempdir()?;
    let user = actor("bookkeeper")?;
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(std::slice::from_ref(&user))?,
        Limits::default(),
    )?;
    assert_eq!(tools.tools().len(), 2);
    let access_help = send(
        &tools,
        &user.0,
        "botus_read",
        json!({"action":"help","topic":"access"}),
    )
    .await?;
    let _: Vec<Owner> = serde_json::from_value(access_help.1["examples"]["owners"].clone())?;
    let _: Vec<Audience> = serde_json::from_value(access_help.1["examples"]["audiences"].clone())?;
    let _: DocumentAccess = serde_json::from_value(access_help.1["examples"]["access"].clone())?;
    let operation_help = send(
        &tools,
        &user.0,
        "botus_read",
        json!({"action":"help","topic":"operations"}),
    )
    .await?;
    let _: Vec<botus_1_2_3::Operation> =
        serde_json::from_value(operation_help.1["operations"].clone())?;
    assert_eq!(
        tools.tool_mutation_class("botus_read"),
        ToolMutationClass::ReadOnly
    );
    assert_eq!(
        tools.tool_mutation_class("botus_apply"),
        ToolMutationClass::Mutating
    );
    assert_authority_unavailable(
        &tools,
        &ToolDispatchContext::default(),
        "botus_read",
        json!({"action":"help"}),
    )
    .await?;
    for forged in ["actor", "realm", "ancestry", "administrator", "path"] {
        let mut request = json!({"action":"list"});
        request[forged] = json!("pretend");
        let (error, result) = send(&tools, &user.0, "botus_read", request).await?;
        assert!(error);
        assert_eq!(result["error"]["code"], "invalid_input");
    }
    Ok(())
}

#[tokio::test]
async fn unavailable_authority_is_infrastructure_feedback_and_sibling_remains_usable() -> TestResult
{
    let temporary = tempfile::tempdir()?;
    let permitted = actor("permitted-sibling")?;
    let unavailable = actor("unpublished-identity")?;
    // This fixture models the resolver result, not a real MobKit cancellation.
    // Both calls have valid origin sessions; only the sibling resolves authority.
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(std::slice::from_ref(&permitted))?,
        Limits::default(),
    )?;
    let request =
        json!({"action":"create","request_id":"unavailable-floppy","title":"No ghost workbooks"});
    assert_authority_unavailable(&tools, &unavailable.0, "botus_apply", request.clone()).await?;
    assert_authority_unavailable(
        &tools,
        &unavailable.0,
        "botus_read",
        json!({"action":"help"}),
    )
    .await?;
    let (error, empty) = send(&tools, &permitted.0, "botus_read", json!({"action":"list"})).await?;
    assert!(!error, "{empty}");
    assert_eq!(empty["documents"], json!([]));
    let (error, created) = send(&tools, &permitted.0, "botus_apply", request.clone()).await?;
    assert!(!error, "{created}");
    assert_eq!(created["replayed"], false);
    assert_authority_unavailable(&tools, &unavailable.0, "botus_apply", request).await?;
    assert_authority_unavailable(
        &tools,
        &unavailable.0,
        "botus_read",
        json!({"action":"read","id":created["receipt"]["document_id"]}),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn native_edit_is_atomic_and_retry_precedes_stale_revision_check() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let user = actor("bookkeeper")?;
    let authority = authority(std::slice::from_ref(&user))?;
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority.clone(),
        Limits::default(),
    )?;
    let first = create(&tools, &user.0).await?;
    let request = json!({"action":"edit","request_id":"calculate","id":first["document_id"],"revision":first["revision"],"operations":[{"op":"formula","at":"C1","formula":"=A1*12"}],"select":{"range":"A1:C1"}});
    let (error, edited) = send(&tools, &user.0, "botus_apply", request.clone()).await?;
    assert!(!error, "{edited}");
    assert_eq!(edited["selected"]["rows"][0][2], 12.0);
    let retry = send(&tools, &user.0, "botus_apply", request.clone()).await?;
    assert!(!retry.0);
    assert_eq!(retry.1["receipt"], edited["receipt"]);
    assert_eq!(retry.1["replayed"], true);
    assert!(retry.1.get("selected").is_none());
    let mut changed = request;
    changed["operations"][0]["formula"] = json!("=42");
    assert_eq!(
        send(&tools, &user.0, "botus_apply", changed).await?.1["error"]["code"],
        "request_id_reused"
    );
    let rejected = send(&tools, &user.0, "botus_apply", json!({"action":"edit","request_id":"bad-batch","id":first["document_id"],"revision":edited["receipt"]["revision"],"operations":[{"op":"put","at":"A1","values":[[99]]},{"op":"formula","at":"D1","formula":"=1/0"}]})).await?;
    assert!(rejected.0);
    let current = send(
        &tools,
        &user.0,
        "botus_read",
        json!({"action":"read","id":first["document_id"],"query":{"range":"A1:D1"}}),
    )
    .await?;
    assert_eq!(current.1["revision"], edited["receipt"]["revision"]);
    assert_eq!(current.1["selected"]["rows"][0][0], 1.0);
    assert_eq!(current.1["selected"]["rows"][0][3], Value::Null);
    assert!(authority.calls.load(Ordering::SeqCst) >= 6);
    Ok(())
}

#[tokio::test]
async fn restart_recovers_workbook_access_and_exact_retry_receipt() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("host.sqlite3");
    let owner = actor("owner")?;
    let reader = actor("reader")?;
    let editor = actor("editor")?;
    let stranger = actor("stranger")?;
    let authority = authority(&[
        owner.clone(),
        reader.clone(),
        editor.clone(),
        stranger.clone(),
    ])?;
    let first_tools = tools(&path, authority.clone(), Limits::default())?;
    let first = create(&first_tools, &owner.0).await?;
    let grant = |name: &str, role: &str| json!({"audience":{"kind":"agent","value":{"principal":{"kind":"agent","value":name},"reach":"forks"}},"role":role});
    let sharing = json!({"action":"share","request_id":"access","id":first["document_id"],"revision":first["revision"],"access":{"grants":[grant("reader","reader"),grant("editor","editor")]}});
    let shared = send(&first_tools, &owner.0, "botus_apply", sharing.clone()).await?;
    assert!(!shared.0, "{}", shared.1);
    drop(first_tools);
    let reopened = tools(&path, authority, Limits::default())?;
    assert_eq!(
        send(&reopened, &owner.0, "botus_apply", sharing).await?.1["receipt"],
        shared.1["receipt"]
    );
    let id = first["document_id"].clone();
    let read = json!({"action":"read","id":id,"query":{"range":"A1:B1"}});
    let selected = send(&reopened, &reader.0, "botus_read", read.clone()).await?;
    assert!(!selected.0);
    assert_eq!(selected.1["selected"]["rows"][0][1], "Unicode: räksmörgås");
    let hidden = send(&reopened, &stranger.0, "botus_read", read).await?;
    let missing = send(
        &reopened,
        &stranger.0,
        "botus_read",
        json!({"action":"read","id":"missing"}),
    )
    .await?;
    assert_eq!(hidden, missing);
    assert_eq!(
        send(
            &reopened,
            &stranger.0,
            "botus_read",
            json!({"action":"list"})
        )
        .await?
        .1["documents"],
        json!([])
    );
    let edit = json!({"action":"edit","request_id":"change","id":id,"revision":shared.1["receipt"]["revision"],"operations":[{"op":"put","at":"A2","values":[[1989]]}]});
    assert!(
        send(&reopened, &reader.0, "botus_apply", edit.clone())
            .await?
            .0
    );
    let edited = send(&reopened, &editor.0, "botus_apply", edit).await?;
    assert!(!edited.0, "{}", edited.1);
    let delete = json!({"action":"delete","request_id":"dispose","id":id,"revision":edited.1["receipt"]["revision"]});
    assert!(
        send(&reopened, &editor.0, "botus_apply", delete.clone())
            .await?
            .0
    );
    let deleted = send(&reopened, &owner.0, "botus_apply", delete.clone()).await?;
    assert!(!deleted.0);
    assert_eq!(deleted.1["receipt"]["deleted"], true);
    assert_eq!(
        send(&reopened, &owner.0, "botus_apply", delete).await?.1["receipt"],
        deleted.1["receipt"]
    );
    Ok(())
}

#[tokio::test]
async fn shared_dispatcher_resolves_fork_spawn_and_revocation_each_time() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let owner = actor("parent")?;
    let mut fork = actor("fork")?;
    let mut spawn = actor("spawn")?;
    for (child, kind) in [(&mut fork, EdgeKind::Fork), (&mut spawn, EdgeKind::Spawn)] {
        child.1 = HostAccessContext::new(
            "beige-realm",
            child.1.principal().clone(),
            VerifiedLineage::derived(vec![LineageLink {
                child: child.1.principal().clone(),
                parent: owner.1.principal().clone(),
                kind,
                ceiling: Some(Role::Reader),
            }])?,
            HostPolicy::ALLOW,
        )?;
    }
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(&[owner.clone(), fork.clone(), spawn.clone()])?,
        Limits::default(),
    )?;
    let first = create(&tools, &owner.0).await?;
    let read = json!({"action":"read","id":first["document_id"]});
    assert!(!send(&tools, &fork.0, "botus_read", read.clone()).await?.0);
    assert!(send(&tools, &spawn.0, "botus_read", read.clone()).await?.0);
    let edit = json!({"action":"edit","request_id":"fork-write","id":first["document_id"],"revision":first["revision"],"operations":[]});
    assert!(send(&tools, &fork.0, "botus_apply", edit).await?.0);
    let revoked = send(&tools, &owner.0, "botus_apply", json!({"action":"share","request_id":"revoke","id":first["document_id"],"revision":first["revision"],"access":{"owner_reach":"self_only"}})).await?;
    assert!(!revoked.0);
    assert!(send(&tools, &fork.0, "botus_read", read).await?.0);
    Ok(())
}

#[tokio::test]
async fn concurrent_edits_have_one_winner_and_large_output_never_commits() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let user = actor("bookkeeper")?;
    let limits = Limits {
        max_response_bytes: 16 * 1024,
        ..Limits::default()
    };
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(std::slice::from_ref(&user))?,
        limits,
    )?;
    let first = create(&tools, &user.0).await?;
    let request = |id: &str, value: Value| json!({"action":"edit","request_id":id,"id":first["document_id"],"revision":first["revision"],"operations":[{"op":"put","at":"A1","values":[[value]]}]});
    let mut huge = request("giant-output", json!("\"".repeat(5000)));
    huge["select"] = json!({"range":"A1"});
    let failed = send(&tools, &user.0, "botus_apply", huge).await?;
    assert!(failed.0);
    assert_eq!(failed.1["error"]["code"], "limit");
    let (left, right) = tokio::join!(
        send(&tools, &user.0, "botus_apply", request("left", json!(2))),
        send(&tools, &user.0, "botus_apply", request("right", json!(3)))
    );
    let left = left?;
    let right = right?;
    assert_ne!(left.0, right.0);
    let loser = if left.0 { left } else { right };
    assert_eq!(loser.1["error"]["code"], "conflict");
    Ok(())
}

/// Forces both duplicate edits to finish calculating before either commits.
struct SimultaneousStore {
    inner: SqliteExtensionDocumentStore,
    gate: tokio::sync::Barrier,
}
#[async_trait]
impl ExtensionDocumentStore for SimultaneousStore {
    async fn get(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        id: &DocumentId,
    ) -> meerkat_mobkit::extension_state::documents::Result<Document> {
        self.inner.get(scope, caller, id).await
    }
    async fn list(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: ListRequest,
    ) -> meerkat_mobkit::extension_state::documents::Result<DocumentPage> {
        self.inner.list(scope, caller, request).await
    }
    async fn lookup_receipt(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
    ) -> meerkat_mobkit::extension_state::documents::Result<Option<MutationReceipt>> {
        self.inner.lookup_receipt(scope, caller, request).await
    }
    async fn mutate(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
        mutation: Mutation,
    ) -> meerkat_mobkit::extension_state::documents::Result<MutationOutcome> {
        if matches!(mutation, Mutation::Replace { .. }) {
            tokio::time::timeout(std::time::Duration::from_secs(10), self.gate.wait())
                .await
                .map_err(|_| Error::Storage("test synchronization timed out".into()))?;
        }
        self.inner.mutate(scope, caller, request, mutation).await
    }
}

#[tokio::test]
async fn concurrent_duplicate_returns_selected_cells_only_for_actual_commit() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let user = actor("bookkeeper")?;
    let store = SimultaneousStore {
        inner: SqliteExtensionDocumentStore::open(temporary.path().join("host.sqlite3"))?,
        gate: tokio::sync::Barrier::new(2),
    };
    let documents = DocumentService::new("beige-realm", NAMESPACE, Arc::new(store))?;
    let tools = BotusToolFactory::default()
        .build(ToolBundleContext {
            documents,
            caller_resolver: authority(std::slice::from_ref(&user))?,
        })
        .map_err(std::io::Error::other)?;
    let first = create(&tools, &user.0).await?;
    let request = json!({"action":"edit","request_id":"same-edit","id":first["document_id"],"revision":first["revision"],"operations":[{"op":"formula","at":"C1","formula":"=A1*12"}],"select":{"range":"C1"}});
    let (left, right) = tokio::join!(
        send(&tools, &user.0, "botus_apply", request.clone()),
        send(&tools, &user.0, "botus_apply", request)
    );
    let left = left?;
    let right = right?;
    assert!(!left.0, "{}", left.1);
    assert!(!right.0, "{}", right.1);
    assert_eq!(left.1["receipt"], right.1["receipt"]);
    assert_ne!(left.1["replayed"], right.1["replayed"]);
    for (_, response) in [left, right] {
        if response["replayed"] == true {
            assert!(response.get("selected").is_none());
            assert!(response.get("changes").is_none());
        } else {
            assert_eq!(response["selected"]["rows"][0][0], 12.0);
        }
    }
    Ok(())
}

/// Pauses an initial missing receipt, allowing commit/delete before the read.
struct DeleteRaceStore {
    inner: SqliteExtensionDocumentStore,
    pause_next_miss: AtomicBool,
    missed: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
#[async_trait]
impl ExtensionDocumentStore for DeleteRaceStore {
    async fn get(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        id: &DocumentId,
    ) -> meerkat_mobkit::extension_state::documents::Result<Document> {
        self.inner.get(scope, caller, id).await
    }
    async fn list(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: ListRequest,
    ) -> meerkat_mobkit::extension_state::documents::Result<DocumentPage> {
        self.inner.list(scope, caller, request).await
    }
    async fn lookup_receipt(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
    ) -> meerkat_mobkit::extension_state::documents::Result<Option<MutationReceipt>> {
        let result = self.inner.lookup_receipt(scope, caller, request).await?;
        if result.is_none() && self.pause_next_miss.swap(false, Ordering::SeqCst) {
            self.missed.notify_one();
            self.resume.notified().await;
        }
        Ok(result)
    }
    async fn mutate(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
        mutation: Mutation,
    ) -> meerkat_mobkit::extension_state::documents::Result<MutationOutcome> {
        self.inner.mutate(scope, caller, request, mutation).await
    }
}

#[tokio::test]
async fn duplicate_edit_after_concurrent_delete_recovers_only_authorized_receipt() -> TestResult {
    for revoke in [false, true] {
        let temporary = tempfile::tempdir()?;
        let owner = actor("owner")?;
        let editor = actor("editor")?;
        let store = Arc::new(DeleteRaceStore {
            inner: SqliteExtensionDocumentStore::open(temporary.path().join("host.sqlite3"))?,
            pause_next_miss: AtomicBool::new(false),
            missed: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let documents = DocumentService::new("beige-realm", NAMESPACE, store.clone())?;
        let tools = BotusToolFactory::default()
            .build(ToolBundleContext {
                documents,
                caller_resolver: authority(&[owner.clone(), editor.clone()])?,
            })
            .map_err(std::io::Error::other)?;
        let first = create(&tools, &owner.0).await?;
        let shared = send(&tools, &owner.0, "botus_apply", json!({"action":"share","request_id":"share-editor","id":first["document_id"],"revision":first["revision"],"access":{"grants":[{"audience":{"kind":"agent","value":{"principal":{"kind":"agent","value":"editor"}}},"role":"editor"}]}})).await?;
        assert!(!shared.0, "{}", shared.1);
        let request = json!({"action":"edit","request_id":"edit-delete-race","id":first["document_id"],"revision":shared.1["receipt"]["revision"],"operations":[{"op":"put","at":"C1","values":[[123]]}],"select":{"range":"C1"}});
        store.pause_next_miss.store(true, Ordering::SeqCst);
        let duplicate = send(&tools, &editor.0, "botus_apply", request.clone());
        let original_then_delete = async {
            store.missed.notified().await;
            let original = send(&tools, &editor.0, "botus_apply", request).await?;
            assert!(!original.0, "{}", original.1);
            let mut revision = original.1["receipt"]["revision"].clone();
            if revoke {
                let revoked = send(&tools, &owner.0, "botus_apply", json!({"action":"share","request_id":"revoke-editor","id":first["document_id"],"revision":revision,"access":{}})).await?;
                assert!(!revoked.0, "{}", revoked.1);
                revision = revoked.1["receipt"]["revision"].clone();
            }
            let deleted = send(&tools, &owner.0, "botus_apply", json!({"action":"delete","request_id":"delete-after-edit","id":first["document_id"],"revision":revision})).await?;
            assert!(!deleted.0, "{}", deleted.1);
            store.resume.notify_one();
            TestResult::Ok(original.1["receipt"].clone())
        };
        let (duplicate, original) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::join!(duplicate, original_then_delete)
            })
            .await?;
        let duplicate = duplicate?;
        let original = original?;
        if revoke {
            assert!(duplicate.0);
            assert_eq!(duplicate.1["error"]["code"], "unavailable");
        } else {
            assert!(!duplicate.0, "{}", duplicate.1);
            assert_eq!(duplicate.1["receipt"], original);
            assert_eq!(duplicate.1["replayed"], true);
            assert!(duplicate.1.get("changes").is_none());
            assert!(duplicate.1.get("selected").is_none());
        }
    }
    Ok(())
}
