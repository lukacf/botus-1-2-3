#![cfg(feature = "meta")]
//! Adapter tests use the real SQLite backend and a deliberately explicit test
//! authority. Runtime lineage and full Meta host restart need host tests too.

#[path = "support/help_chapters.rs"]
mod help_chapters;

use async_trait::async_trait;
use botus_1_2_3::{
    Limits,
    native::{
        ApplyAction, BotusToolFactory, MAX_TOOL_DEFINITION_BYTES, NAMESPACE, ReadAction,
        tool_definitions,
    },
};
use meerkat_core::{
    AgentToolDispatcher, SessionId, ToolCallView, ToolDispatchContext, ToolError,
    ToolMutationClass, ToolProvenance, ToolSourceKind,
};
use meerkat_meta::extension_state::{
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
    ) -> meerkat_meta::extension_state::documents::Result<HostAccessContext> {
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

#[test]
fn client_schemas_accept_documented_nullable_arguments() -> TestResult {
    let definitions = tool_definitions();
    for (name, requests) in [
        (
            "botus_read",
            vec![
                json!({"action":"list","after":null}),
                json!({"action":"read","id":"workbook","query":{
                    "range":null,"columns":null,"filter":null
                }}),
            ],
        ),
        (
            "botus_apply",
            vec![
                json!({"action":"create","request_id":"create","title":"Budget",
                    "owner":null,"select":null}),
                json!({"action":"edit","request_id":"edit","id":"workbook",
                    "revision":"current","operations":[{"op":"clear","range":"A1"}],
                    "select":{"range":null,"columns":null,"filter":null}}),
                json!({"action":"transfer","request_id":"transfer","id":"workbook",
                    "revision":"current","owner":{"kind":"realm"},"access":null}),
            ],
        ),
    ] {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == name)
            .ok_or("missing tool definition")?;
        let validator = jsonschema::validator_for(&definition.input_schema)
            .map_err(|error| error.to_string())?;
        for request in requests {
            // Exercise the same JSON Schema contract an MCP client sees,
            // then the actual parser. Both must accept the manual's inputs.
            assert!(validator.is_valid(&request), "{name} rejected {request}");
            match name {
                "botus_read" => {
                    let _: ReadAction = serde_json::from_value(request)?;
                }
                _ => {
                    let _: ApplyAction = serde_json::from_value(request)?;
                }
            }
        }
        let wrong = if name == "botus_read" {
            json!({"action":"read","id":"workbook","query":null})
        } else {
            json!({"action":"create","request_id":"create","title":"Budget","select":false})
        };
        assert!(!validator.is_valid(&wrong));
    }
    Ok(())
}

#[tokio::test]
async fn schemas_are_compact_and_authority_is_never_an_argument() -> TestResult {
    let definitions = tool_definitions();
    let serialized_definitions = serde_json::to_vec(&definitions)?;
    assert_eq!(definitions.len(), 2);
    assert!(serialized_definitions.len() <= MAX_TOOL_DEFINITION_BYTES);
    let wire_definitions: Vec<Value> = serde_json::from_slice(&serialized_definitions)?;
    let bundle_provenance = ToolProvenance::new(ToolSourceKind::RustBundle, NAMESPACE);
    for ((definition, wire), name) in definitions
        .iter()
        .zip(&wire_definitions)
        .zip(["botus_read", "botus_apply"])
    {
        assert_eq!(wire["name"], name);
        assert_eq!(definition.provenance.as_ref(), Some(&bundle_provenance));
        assert_eq!(
            wire["provenance"],
            json!({"kind":"rust_bundle","source_id":NAMESPACE})
        );
    }
    let temporary = tempfile::tempdir()?;
    let user = actor("bookkeeper")?;
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(std::slice::from_ref(&user))?,
        Limits::default(),
    )?;
    assert_eq!(tools.tools().len(), 2);
    for definition in tools.tools().iter() {
        assert_eq!(definition.provenance.as_ref(), Some(&bundle_provenance));
    }
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
    let worker: Principal = serde_json::from_value(
        access_help.1["examples"]["worker_identity"]["caller_principal"].clone(),
    )?;
    assert!(matches!(worker, Principal::Worker { .. }));
    let ApplyAction::Share { access, .. } =
        serde_json::from_value(access_help.1["examples"]["share_with_worker"].clone())?
    else {
        return Err("expected a worker sharing example".into());
    };
    assert_eq!(
        access.grants,
        vec![Grant {
            audience: Audience::Agent {
                principal: worker,
                reach: Reach::SelfOnly,
            },
            role: Role::Reader,
        }]
    );
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
    for action in ["identity", "list"] {
        for forged in [
            "actor",
            "realm",
            "ancestry",
            "administrator",
            "path",
            "caller_principal",
            "principal",
            "mob_id",
            "member_id",
            "creation_id",
            "session_id",
            "id",
            "query",
        ] {
            let mut request = json!({"action":action});
            request[forged] = json!("pretend");
            let (error, result) = send(&tools, &user.0, "botus_read", request).await?;
            assert!(error, "{action} accepted {forged}");
            assert_eq!(result["error"]["code"], "invalid_input");
        }
    }
    Ok(())
}

#[tokio::test]
async fn full_help_chapters_fit_minimum_response_budget_and_refuse_oversized_envelopes()
-> TestResult {
    let temporary = tempfile::tempdir()?;
    let user = actor("manual-reader")?;
    let max_response_bytes = 16 * 1024;
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority(std::slice::from_ref(&user))?,
        Limits {
            max_response_bytes,
            ..Limits::default()
        },
    )?;
    let definitions = tool_definitions();
    let read = definitions
        .iter()
        .find(|definition| definition.name == "botus_read")
        .ok_or("missing read definition")?;
    let validator =
        jsonschema::validator_for(&read.input_schema).map_err(|error| error.to_string())?;
    for (topic, field, chapter) in help_chapters::CHAPTERS {
        let request = json!({"action":"help","topic":topic});
        assert!(validator.is_valid(&request), "schema rejects {topic}");
        let raw = serde_json::value::to_raw_value(&request)?;
        let outcome = tools
            .dispatch_with_context(
                ToolCallView {
                    id: "full-manual-chapter",
                    name: "botus_read",
                    args: &raw,
                },
                &user.0,
            )
            .await?;
        assert!(!outcome.result.is_error, "help failed for {topic}");
        assert!(serde_json::to_vec(&outcome.result)?.len() <= max_response_bytes);
        let result: Value = serde_json::from_str(&outcome.result.text_content())?;
        assert_eq!(result[field].as_str(), Some(chapter), "incomplete {topic}");
        if topic == "limits" {
            assert_eq!(result["limits"]["response_bytes"], max_response_bytes);
        }
    }

    // The complete chapter fits, but an escaped tool-call ID can make its
    // enclosing result too large. The bounded error must replace the whole
    // response, never return a silently shortened manual.
    let large_call_id = "\\".repeat(6 * 1024);
    let raw = serde_json::value::to_raw_value(&json!({"action":"help","topic":"apply"}))?;
    let outcome = tools
        .dispatch_with_context(
            ToolCallView {
                id: &large_call_id,
                name: "botus_read",
                args: &raw,
            },
            &user.0,
        )
        .await?;
    assert!(outcome.result.is_error);
    assert!(serde_json::to_vec(&outcome.result)?.len() <= max_response_bytes);
    let result: Value = serde_json::from_str(&outcome.result.text_content())?;
    assert_eq!(result["error"]["code"], "limit");
    assert_eq!(
        result["error"]["message"],
        "Complete help chapter exceeds response byte limit; the host must allow a larger response"
    );
    assert!(result.get("help").is_none());
    Ok(())
}

#[tokio::test]
async fn identity_reports_current_parent_and_worker_principals_without_documents() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let parent = actor("parent-bookkeeper")?;
    // These are trusted fixture facts, never inferred from the dispatch session.
    // Maximal escaped identifiers also exercise the bounded wire response.
    let worker = Principal::Worker {
        mob_id: "\\".repeat(MAX_IDENTIFIER_BYTES),
        member_id: "\"".repeat(MAX_IDENTIFIER_BYTES),
        creation_id: "c".repeat(MAX_IDENTIFIER_BYTES),
    };
    let child = (
        ToolDispatchContext::default().with_runtime_identity(SessionId::new(), None),
        HostAccessContext::new(
            "beige-realm",
            worker.clone(),
            VerifiedLineage::derived(vec![LineageLink {
                child: worker,
                parent: parent.1.principal().clone(),
                kind: EdgeKind::Spawn,
                ceiling: Some(Role::Editor),
            }])?,
            HostPolicy::ALLOW,
        )?,
    );
    let authority = authority(&[parent.clone(), child.clone()])?;
    let limits = Limits {
        max_response_bytes: 16 * 1024,
        ..Limits::default()
    };
    let max_response_bytes = limits.max_response_bytes;
    let tools = tools(
        &temporary.path().join("host.sqlite3"),
        authority.clone(),
        limits,
    )?;
    let request = serde_json::value::to_raw_value(&json!({"action":"identity"}))?;
    for caller in [&parent, &child, &parent] {
        let outcome = tools
            .dispatch_with_context(
                ToolCallView {
                    id: "identity-call",
                    name: "botus_read",
                    args: &request,
                },
                &caller.0,
            )
            .await?;
        assert!(!outcome.result.is_error);
        assert!(serde_json::to_vec(&outcome.result)?.len() <= max_response_bytes);
        let encoded = serde_json::to_value(&outcome.result)?;
        let content = encoded["content"].as_str().ok_or("expected text result")?;
        let result: Value = serde_json::from_str(content)?;
        assert_eq!(result, json!({"caller_principal":caller.1.principal()}));
        let reported: Principal = serde_json::from_value(result["caller_principal"].clone())?;
        assert_eq!(&reported, caller.1.principal());
    }
    assert_eq!(authority.calls.load(Ordering::SeqCst), 3);

    // Oversized envelope bytes cannot bypass the same bound used by other reads.
    let large_call_id = "\"".repeat(max_response_bytes);
    let rejected = tools
        .dispatch_with_context(
            ToolCallView {
                id: &large_call_id,
                name: "botus_read",
                args: &request,
            },
            &child.0,
        )
        .await;
    assert!(matches!(
        rejected,
        Err(ToolError::ExecutionFailed { message }) if message == "Botus response exceeds byte limit"
    ));
    for caller in [&parent, &child] {
        let (error, page) = send(&tools, &caller.0, "botus_read", json!({"action":"list"})).await?;
        assert!(!error, "{page}");
        assert_eq!(page["documents"], json!([]));
    }
    Ok(())
}

#[tokio::test]
async fn unavailable_authority_is_infrastructure_feedback_and_sibling_remains_usable() -> TestResult
{
    let temporary = tempfile::tempdir()?;
    let permitted = actor("permitted-sibling")?;
    let unavailable = actor("unpublished-identity")?;
    // This fixture models the resolver result, not a real Meta cancellation.
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
    assert_authority_unavailable(
        &tools,
        &unavailable.0,
        "botus_read",
        json!({"action":"identity"}),
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
async fn ownership_transfer_requires_management_and_preserves_acl_for_new_owner() -> TestResult {
    // The host attests one destination. This does not grant any caller the
    // authority to manage the current owner; the real store must still check it.
    struct TransferAuthority {
        callers: Arc<TestAuthority>,
        destination: Owner,
    }
    #[async_trait]
    impl ToolCallerResolver for TransferAuthority {
        async fn resolve(
            &self,
            context: &ToolDispatchContext,
            requested_owner: Option<&Owner>,
        ) -> meerkat_meta::extension_state::documents::Result<HostAccessContext> {
            if requested_owner.is_some_and(|owner| owner != &self.destination) {
                return Err(Error::NotFound);
            }
            Ok(self
                .callers
                .resolve(context, None)
                .await?
                .with_valid_owners([self.destination.clone()].into()))
        }
    }

    let temporary = tempfile::tempdir()?;
    let owner = actor("transfer-owner")?;
    let editor = actor("transfer-editor")?;
    let recipient = actor("transfer-recipient")?;
    let restricted_owner = (
        ToolDispatchContext::default().with_runtime_identity(SessionId::new(), None),
        HostAccessContext::new(
            "beige-realm",
            owner.1.principal().clone(),
            VerifiedLineage::root(),
            HostPolicy {
                read: true,
                edit: true,
                manage: false,
            },
        )?,
    );
    let destination = Owner::Agent(recipient.1.principal().clone());
    let documents = DocumentService::new(
        "beige-realm",
        NAMESPACE,
        Arc::new(SqliteExtensionDocumentStore::open(
            temporary.path().join("host.sqlite3"),
        )?),
    )?;
    let tools = BotusToolFactory::default()
        .build(ToolBundleContext {
            documents,
            caller_resolver: Arc::new(TransferAuthority {
                callers: authority(&[
                    owner.clone(),
                    restricted_owner.clone(),
                    editor.clone(),
                    recipient.clone(),
                ])?,
                destination: destination.clone(),
            }),
        })
        .map_err(std::io::Error::other)?;
    let access = DocumentAccess {
        owner_reach: Reach::SelfOnly,
        grants: vec![Grant {
            audience: Audience::Agent {
                principal: editor.1.principal().clone(),
                reach: Reach::SelfOnly,
            },
            role: Role::Editor,
        }],
        ..Default::default()
    };
    let (error, created) = send(
        &tools,
        &owner.0,
        "botus_apply",
        json!({"action":"create","request_id":"transfer-create","title":"Hand over the floppy",
            "access":access,"operations":[{"op":"put","at":"A1","values":[[1]]}]}),
    )
    .await?;
    assert!(!error, "{created}");
    let id = created["receipt"]["document_id"].clone();
    let (error, edited) = send(
        &tools,
        &editor.0,
        "botus_apply",
        json!({"action":"edit","request_id":"editor-control","id":id,
            "revision":created["receipt"]["revision"],
            "operations":[{"op":"put","at":"A1","values":[[2]]}]}),
    )
    .await?;
    assert!(!error, "{edited}");
    let revision = edited["receipt"]["revision"].clone();
    let transfer = json!({"action":"transfer","request_id":"handoff","id":id,
        "revision":revision,"owner":destination});

    // A data editor and even the owner under a no-management host policy
    // cannot transfer. A valid destination alone grants neither permission.
    for caller in [&editor, &restricted_owner] {
        let (error, refused) = send(&tools, &caller.0, "botus_apply", transfer.clone()).await?;
        assert!(error, "{refused}");
        assert_eq!(refused["error"]["code"], "unavailable");
    }
    let mut stale = transfer.clone();
    stale["request_id"] = json!("stale-handoff");
    stale["revision"] = created["receipt"]["revision"].clone();
    let (error, refused) = send(&tools, &owner.0, "botus_apply", stale).await?;
    assert!(error, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict");
    let describe = json!({"action":"describe","id":id});
    let (error, unchanged) = send(&tools, &owner.0, "botus_read", describe.clone()).await?;
    assert!(!error, "{unchanged}");
    assert_eq!(
        unchanged["owner"],
        json!(Owner::Agent(owner.1.principal().clone()))
    );
    assert_eq!(unchanged["revision"], revision);
    assert_eq!(unchanged["access"], json!(access));

    // Omitting access preserves the complete ACL while changing ownership.
    let (error, transferred) = send(&tools, &owner.0, "botus_apply", transfer.clone()).await?;
    assert!(!error, "{transferred}");
    assert_eq!(transferred["receipt"]["document_id"], id);
    assert_ne!(transferred["receipt"]["revision"], revision);
    assert_eq!(transferred["receipt"]["deleted"], false);
    assert_eq!(transferred["replayed"], false);
    assert!(transferred.get("changes").is_none());
    assert!(transferred.get("selected").is_none());
    let (error, current) = send(&tools, &recipient.0, "botus_read", describe.clone()).await?;
    assert!(!error, "{current}");
    assert_eq!(current["owner"], json!(destination));
    assert_eq!(current["revision"], transferred["receipt"]["revision"]);
    assert_eq!(current["access"], json!(access));
    for caller in [&recipient, &editor] {
        let (error, read) = send(
            &tools,
            &caller.0,
            "botus_read",
            json!({"action":"read","id":id,"query":{"range":"A1"}}),
        )
        .await?;
        assert!(!error, "{read}");
        assert_eq!(read["selected"]["rows"], json!([[2.0]]));
    }
    // The former owner loses both data access and authority to replay its
    // successful administrative receipt once the transfer has committed.
    for (tool, request) in [("botus_read", describe.clone()), ("botus_apply", transfer)] {
        let (error, refused) = send(&tools, &owner.0, tool, request).await?;
        assert!(error, "{refused}");
        assert_eq!(refused["error"]["code"], "unavailable");
    }

    // The recipient gains administrative authority, not just data access.
    let private = DocumentAccess {
        owner_reach: Reach::SelfOnly,
        ..Default::default()
    };
    let (error, shared) = send(
        &tools,
        &recipient.0,
        "botus_apply",
        json!({"action":"share","request_id":"recipient-management-control","id":id,
            "revision":transferred["receipt"]["revision"],"access":private}),
    )
    .await?;
    assert!(!error, "{shared}");
    assert_ne!(
        shared["receipt"]["revision"],
        transferred["receipt"]["revision"]
    );
    let (error, current) = send(&tools, &recipient.0, "botus_read", describe.clone()).await?;
    assert!(!error, "{current}");
    assert_eq!(current["owner"], json!(destination));
    assert_eq!(current["revision"], shared["receipt"]["revision"]);
    assert_eq!(current["access"], json!(private));
    let (error, refused) = send(&tools, &editor.0, "botus_read", describe).await?;
    assert!(error, "{refused}");
    assert_eq!(refused["error"]["code"], "unavailable");
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
    ) -> meerkat_meta::extension_state::documents::Result<Document> {
        self.inner.get(scope, caller, id).await
    }
    async fn list(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: ListRequest,
    ) -> meerkat_meta::extension_state::documents::Result<DocumentPage> {
        self.inner.list(scope, caller, request).await
    }
    async fn lookup_receipt(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
    ) -> meerkat_meta::extension_state::documents::Result<Option<MutationReceipt>> {
        self.inner.lookup_receipt(scope, caller, request).await
    }
    async fn mutate(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
        mutation: Mutation,
    ) -> meerkat_meta::extension_state::documents::Result<MutationOutcome> {
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
    ) -> meerkat_meta::extension_state::documents::Result<Document> {
        self.inner.get(scope, caller, id).await
    }
    async fn list(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: ListRequest,
    ) -> meerkat_meta::extension_state::documents::Result<DocumentPage> {
        self.inner.list(scope, caller, request).await
    }
    async fn lookup_receipt(
        &self,
        scope: &DocumentScope,
        caller: &HostAccessContext,
        request: &RequestIdentity,
    ) -> meerkat_meta::extension_state::documents::Result<Option<MutationReceipt>> {
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
    ) -> meerkat_meta::extension_state::documents::Result<MutationOutcome> {
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
