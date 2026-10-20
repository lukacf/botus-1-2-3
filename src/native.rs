//! Optional Meta tools. The host owns the filing cabinet; Botus owns F9.
//!
//! A replay returns the original durable receipt, without recalculating a
//! selection against newer state. Ask `botus_read` for a fresh selection.

use crate::{ApplyResult, EngineError, Limits, Operation, ReadRequest, ReadResult, Workbook};
use async_trait::async_trait;
use documents::{
    DocumentAccess, DocumentContent, DocumentId, HostAccessContext, Mutation, MutationReceipt,
    NewDocument, Owner, RequestIdentity, Revision,
};
use meerkat_core::{
    AgentToolDispatcher, ToolCallView, ToolDef, ToolDispatchContext, ToolDispatchOutcome,
    ToolError, ToolMutationClass, ToolProvenance, ToolResult, ToolSourceKind,
};
use meerkat_meta::extension_state::{
    ToolBundleContext, ToolBundleFactory, ToolBundleRequirements, documents,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::sync::Arc;

pub const NAMESPACE: &str = "botus-1-2-3";
pub const CONTENT_TYPE: &str = "application/vnd.botus.workbook+json";
pub const MAX_TOOL_DEFINITION_BYTES: usize = 6 * 1024;
pub const MAX_REQUEST_BYTES: usize = documents::MAX_PAYLOAD_BYTES;
const MIN_RESPONSE_BYTES: usize = 16 * 1024;

fn list_limit() -> usize {
    50
}

/// Compact tool requests contain intended actions, never caller authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadAction {
    Identity {},
    List {
        #[serde(default = "list_limit")]
        limit: usize,
        #[serde(default)]
        after: Option<DocumentId>,
    },
    Describe {
        id: DocumentId,
    },
    Read {
        id: DocumentId,
        #[serde(default)]
        query: ReadRequest,
    },
    Help {
        #[serde(default)]
        topic: HelpTopic,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpTopic {
    #[default]
    Overview,
    Read,
    Apply,
    Operations,
    Formulas,
    Access,
    Limits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplyAction {
    Create {
        request_id: String,
        title: String,
        #[serde(default)]
        owner: Option<Owner>,
        #[serde(default)]
        access: DocumentAccess,
        #[serde(default)]
        operations: Vec<Operation>,
        #[serde(default)]
        select: Option<ReadRequest>,
    },
    Edit {
        request_id: String,
        id: DocumentId,
        revision: Revision,
        operations: Vec<Operation>,
        #[serde(default)]
        select: Option<ReadRequest>,
    },
    Share {
        request_id: String,
        id: DocumentId,
        revision: Revision,
        access: DocumentAccess,
    },
    Transfer {
        request_id: String,
        id: DocumentId,
        revision: Revision,
        owner: Owner,
        #[serde(default)]
        access: Option<DocumentAccess>,
    },
    Delete {
        request_id: String,
        id: DocumentId,
        revision: Revision,
    },
}

impl ApplyAction {
    fn request_id(&self) -> &str {
        match self {
            Self::Create { request_id, .. }
            | Self::Edit { request_id, .. }
            | Self::Share { request_id, .. }
            | Self::Transfer { request_id, .. }
            | Self::Delete { request_id, .. } => request_id,
        }
    }
    fn requested_owner(&self) -> Option<&Owner> {
        match self {
            Self::Create { owner, .. } => owner.as_ref(),
            Self::Transfer { owner, .. } => Some(owner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResponse {
    pub receipt: MutationReceipt,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<ApplyResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<ReadResult>,
}

/// Register explicitly with Meta. A default core build does not include this.
#[derive(Debug, Clone, Default)]
pub struct BotusToolFactory {
    pub limits: Limits,
}

impl BotusToolFactory {
    pub fn requirements() -> ToolBundleRequirements {
        ToolBundleRequirements::durable_documents(NAMESPACE, "botus_read", "botus_apply")
    }
}

impl ToolBundleFactory for BotusToolFactory {
    fn build(
        &self,
        context: ToolBundleContext,
    ) -> std::result::Result<Arc<dyn AgentToolDispatcher>, String> {
        if context.documents.scope().namespace() != NAMESPACE {
            return Err("Botus requires its own document namespace".into());
        }
        if self.limits.max_response_bytes < MIN_RESPONSE_BYTES {
            return Err("Botus response limit must be at least 16384 bytes".into());
        }
        let mut limits = self.limits.clone();
        limits.max_document_bytes = limits.max_document_bytes.min(documents::MAX_PAYLOAD_BYTES);
        let definitions = tool_definitions();
        let definition_bytes = serde_json::to_vec(&definitions)
            .map_err(|_| "Cannot serialize Botus tool definitions")?;
        if definition_bytes.len() > MAX_TOOL_DEFINITION_BYTES {
            return Err("Botus tool definitions exceed their context budget".into());
        }
        Ok(Arc::new(BotusTools {
            context,
            limits,
            definitions: definitions.into_iter().map(Arc::new).collect(),
        }))
    }
}

struct BotusTools {
    context: ToolBundleContext,
    limits: Limits,
    definitions: Arc<[Arc<ToolDef>]>,
}

#[derive(Debug)]
enum Failure {
    AuthorityUnavailable,
    Operation { code: &'static str, message: String },
}
type Result<T> = std::result::Result<T, Failure>;

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self::Operation {
            code,
            message: message.into(),
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }
}
impl From<documents::Error> for Failure {
    fn from(error: documents::Error) -> Self {
        match error {
            documents::Error::NotFound => Self::new("unavailable", "Document unavailable"),
            documents::Error::Conflict => Self::new(
                "conflict",
                "Document revision changed; read it and submit a new request ID",
            ),
            documents::Error::RequestIdReused => Self::new(
                "request_id_reused",
                "Request ID already used for different content",
            ),
            documents::Error::AuthorityUnavailable => Self::AuthorityUnavailable,
            documents::Error::Invalid(_) => Self::invalid("Invalid document request"),
            documents::Error::Storage(_) => Self::new("storage", "Document storage failed"),
        }
    }
}
impl From<EngineError> for Failure {
    fn from(error: EngineError) -> Self {
        Self::new("workbook", error.to_string())
    }
}

#[async_trait]
impl AgentToolDispatcher for BotusTools {
    fn tools(&self) -> Arc<[Arc<ToolDef>]> {
        Arc::clone(&self.definitions)
    }
    fn tool_mutation_class(&self, name: &str) -> ToolMutationClass {
        match name {
            "botus_read" => ToolMutationClass::ReadOnly,
            "botus_apply" => ToolMutationClass::Mutating,
            _ => ToolMutationClass::Unknown,
        }
    }
    async fn dispatch(
        &self,
        call: ToolCallView<'_>,
    ) -> std::result::Result<ToolDispatchOutcome, ToolError> {
        self.failure(call.id, Failure::AuthorityUnavailable)
    }
    async fn dispatch_with_context(
        &self,
        call: ToolCallView<'_>,
        context: &ToolDispatchContext,
    ) -> std::result::Result<ToolDispatchOutcome, ToolError> {
        let result = self.execute(call, context).await;
        match result {
            Ok(output) => Ok(output),
            Err(error) => self.failure(call.id, error),
        }
    }
}

impl BotusTools {
    fn output<T: Serialize>(
        &self,
        id: &str,
        body: &T,
        is_error: bool,
    ) -> Result<ToolDispatchOutcome> {
        let content = serde_json::to_string(body)
            .map_err(|_| Failure::new("serialization", "Cannot serialize tool result"))?;
        let result = ToolResult::new(id.to_owned(), content, is_error);
        // Count actual wire bytes, including JSON escaping of the text content.
        let size = serde_json::to_vec(&result)
            .map_err(|_| Failure::new("serialization", "Cannot serialize tool result"))?
            .len();
        if size > self.limits.max_response_bytes {
            return Err(Failure::new(
                "limit",
                "Tool response exceeds byte limit; request a smaller selection",
            ));
        }
        Ok(ToolDispatchOutcome::sync_result(result))
    }
    fn failure(
        &self,
        id: &str,
        error: Failure,
    ) -> std::result::Result<ToolDispatchOutcome, ToolError> {
        let Failure::Operation { code, message } = error else {
            return Err(ToolError::ExecutionFailed {
                message: "Caller authority unavailable".into(),
            });
        };
        let message: String = message.chars().take(512).collect();
        self.output(id, &json!({"error":{"code":code,"message":message}}), true)
            .map_err(|_| ToolError::ExecutionFailed {
                message: "Botus response exceeds byte limit".into(),
            })
    }
    async fn execute(
        &self,
        call: ToolCallView<'_>,
        context: &ToolDispatchContext,
    ) -> Result<ToolDispatchOutcome> {
        if context.origin_session_id().is_none() {
            return Err(Failure::AuthorityUnavailable);
        }
        if call.args.get().len() > MAX_REQUEST_BYTES {
            return Err(Failure::new("limit", "Tool request exceeds byte limit"));
        }
        match call.name {
            "botus_read" => {
                let request: ReadAction = call
                    .parse_args()
                    .map_err(|e| Failure::invalid(e.to_string()))?;
                let caller = self.context.caller_resolver.resolve(context, None).await?;
                self.read(call.id, &caller, request).await
            }
            "botus_apply" => {
                let request: ApplyAction = call
                    .parse_args()
                    .map_err(|e| Failure::invalid(e.to_string()))?;
                let caller = self
                    .context
                    .caller_resolver
                    .resolve(context, request.requested_owner())
                    .await?;
                self.apply(call.id, &caller, request).await
            }
            _ => Err(Failure::invalid("Unknown Botus tool")),
        }
    }
    fn workbook(&self, content: &DocumentContent) -> Result<Workbook> {
        if content.content_type != CONTENT_TYPE
            || content.schema_version != crate::engine::SCHEMA_VERSION
        {
            return Err(Failure::new(
                "version",
                "Unsupported workbook format or version",
            ));
        }
        Workbook::from_json(&content.payload, &self.limits).map_err(Into::into)
    }
    async fn read(
        &self,
        call_id: &str,
        caller: &HostAccessContext,
        request: ReadAction,
    ) -> Result<ToolDispatchOutcome> {
        match request {
            ReadAction::Identity {} => self.output(
                call_id,
                &json!({"caller_principal":caller.principal()}),
                false,
            ),
            ReadAction::List { limit, after } => {
                if limit == 0 || limit > 100 {
                    return Err(Failure::invalid("List limit must be between 1 and 100"));
                }
                let page = self
                    .context
                    .documents
                    .list(caller, documents::ListRequest { limit, after })
                    .await?;
                self.output(call_id, &page, false)
            }
            ReadAction::Describe { id } => {
                let document = self.context.documents.get(caller, &id).await?;
                let book = self.workbook(&document.content)?;
                self.output(call_id, &json!({"id":document.id,"revision":document.revision,"title":document.content.title,"owner":document.owner,"access":document.access,"workbook":book.describe(&self.limits)?}), false)
            }
            ReadAction::Read { id, query } => {
                let document = self.context.documents.get(caller, &id).await?;
                let selected = self
                    .workbook(&document.content)?
                    .read(&query, &self.limits)?;
                self.output(
                    call_id,
                    &json!({"id":document.id,"revision":document.revision,"selected":selected}),
                    false,
                )
            }
            ReadAction::Help { topic } => self
                .output(call_id, &help(topic, &self.limits), false)
                .map_err(|error| match error {
                    Failure::Operation { code: "limit", .. } => Failure::new(
                        "limit",
                        "Complete help chapter exceeds response byte limit; the host must allow a larger response",
                    ),
                    other => other,
                }),
        }
    }
    async fn apply(
        &self,
        call_id: &str,
        caller: &HostAccessContext,
        request: ApplyAction,
    ) -> Result<ToolDispatchOutcome> {
        // Normalize typed defaults and field order, before any state-dependent work.
        let original = serde_json::to_vec(&request)
            .map_err(|_| Failure::invalid("Cannot normalize request"))?;
        let identity = RequestIdentity::new(request.request_id(), &original)?;
        if let Some(receipt) = self
            .context
            .documents
            .lookup_receipt(caller, &identity)
            .await?
        {
            return self.output(
                call_id,
                &ApplyResponse {
                    receipt,
                    replayed: true,
                    changes: None,
                    selected: None,
                },
                false,
            );
        }
        let (mutation, changes, selected) = match request {
            ApplyAction::Create {
                title,
                owner,
                access,
                operations,
                select,
                ..
            } => {
                let mut book = Workbook::new();
                let changes = if operations.is_empty() {
                    None
                } else {
                    Some(book.apply(&operations, &self.limits)?)
                };
                let selected = select
                    .map(|query| book.read(&query, &self.limits))
                    .transpose()?;
                let content = self.content(title, &book)?;
                (
                    Mutation::Create(NewDocument {
                        content,
                        owner,
                        access,
                    }),
                    changes,
                    selected,
                )
            }
            ApplyAction::Edit {
                id,
                revision,
                operations,
                select,
                ..
            } => {
                let document = match self.context.documents.get(caller, &id).await {
                    Ok(document) => document,
                    Err(error) => {
                        // The original request may have committed and the
                        // document then been deleted since our first lookup.
                        // Only the provider can authorize the retained receipt.
                        if let Ok(Some(receipt)) = self
                            .context
                            .documents
                            .lookup_receipt(caller, &identity)
                            .await
                        {
                            return self.output(
                                call_id,
                                &ApplyResponse {
                                    receipt,
                                    replayed: true,
                                    changes: None,
                                    selected: None,
                                },
                                false,
                            );
                        }
                        return Err(error.into());
                    }
                };
                if document.revision != revision {
                    // A duplicate can commit after the first receipt lookup
                    // but before this read. Recover that exact receipt too.
                    if let Some(receipt) = self
                        .context
                        .documents
                        .lookup_receipt(caller, &identity)
                        .await?
                    {
                        return self.output(
                            call_id,
                            &ApplyResponse {
                                receipt,
                                replayed: true,
                                changes: None,
                                selected: None,
                            },
                            false,
                        );
                    }
                    return Err(documents::Error::Conflict.into());
                }
                let mut book = self.workbook(&document.content)?;
                let changes = book.apply(&operations, &self.limits)?;
                let selected = select
                    .map(|query| book.read(&query, &self.limits))
                    .transpose()?;
                let content = self.content(document.content.title, &book)?;
                (
                    Mutation::Replace {
                        id,
                        expected_revision: revision,
                        content,
                    },
                    Some(changes),
                    selected,
                )
            }
            ApplyAction::Share {
                id,
                revision,
                access,
                ..
            } => (
                Mutation::SetAccess {
                    id,
                    expected_revision: revision,
                    access,
                },
                None,
                None,
            ),
            ApplyAction::Transfer {
                id,
                revision,
                owner,
                access,
                ..
            } => (
                Mutation::Transfer {
                    id,
                    expected_revision: revision,
                    owner,
                    access,
                },
                None,
                None,
            ),
            ApplyAction::Delete { id, revision, .. } => (
                Mutation::Delete {
                    id,
                    expected_revision: revision,
                },
                None,
                None,
            ),
        };
        // Provider identifiers are bounded to 512 bytes. Reserve their maximum
        // escaping before committing so a large selection cannot cause a write
        // followed by an avoidable response-limit error.
        let mut response = ApplyResponse {
            receipt: MutationReceipt {
                document_id: DocumentId("\\".repeat(documents::MAX_IDENTIFIER_BYTES)),
                revision: Revision("\\".repeat(documents::MAX_IDENTIFIER_BYTES)),
                deleted: false,
            },
            replayed: false,
            changes,
            selected,
        };
        self.output(call_id, &response, false)?;
        let outcome = self
            .context
            .documents
            .mutate(caller, &identity, mutation)
            .await?;
        response.receipt = outcome.receipt;
        response.replayed = outcome.replayed;
        if outcome.replayed {
            response.changes = None;
            response.selected = None;
        }
        self.output(call_id, &response, false)
    }
    fn content(&self, title: String, book: &Workbook) -> Result<DocumentContent> {
        if title.len() > 1024 {
            return Err(Failure::invalid("Workbook title exceeds 1024 bytes"));
        }
        Ok(DocumentContent {
            title,
            content_type: CONTENT_TYPE.into(),
            schema_version: crate::engine::SCHEMA_VERSION,
            payload: book.to_json(&self.limits)?,
        })
    }
}

/// Descriptions stay small. Full syntax is supplied by the read tool on demand.
pub fn tool_definitions() -> Vec<ToolDef> {
    let query = json!({"type":"object","additionalProperties":false,"properties":{
        "sheet":{"type":"string"},"range":{"type":["string","null"]},"columns":{"type":["array","null"],"items":{"type":"string"}},
        "view":{"enum":["values","formulas"]},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1},
        "filter":{"type":["object","null"],"additionalProperties":false,"required":["column","op","value"],"properties":{"column":{"type":"string"},"op":{"enum":["eq","ne","lt","le","gt","ge","contains"]},"value":{}}}
    }});
    let mut selection = query.clone();
    selection["type"] = json!(["object", "null"]);
    vec![
        ToolDef::new(
            "botus_read",
            "Read durable agent spreadsheets: identity, list, describe, read or help. identity returns your host-resolved caller_principal for sharing. A read returns a bounded matrix and revision. Fetch help topics read/apply/operations/formulas/access/limits for complete reference chapters.",
            json!({"type":"object","additionalProperties":false,"required":["action"],"properties":{
                "action":{"enum":["identity","list","describe","read","help"]},"id":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100},"after":{"type":["string","null"]},"query":query,
                "topic":{"enum":["overview","read","apply","operations","formulas","access","limits"]}
            }}),
        )
        .with_provenance(ToolProvenance::new(ToolSourceKind::RustBundle, NAMESPACE)),
        ToolDef::new(
            "botus_apply",
            "Create, edit, share, transfer or delete a workbook. Use a unique request_id; retries return its receipt. All but create require id and current revision. Edits are atomic. select optionally returns result cells. Strings stay text; formulas use {formula:...}.",
            json!({"type":"object","additionalProperties":false,"required":["action","request_id"],"properties":{
                "action":{"enum":["create","edit","share","transfer","delete"]},"request_id":{"type":"string"},"id":{"type":"string"},"revision":{"type":"string"},"title":{"type":"string"},
                "owner":{"type":["object","null"],"description":"Requested recipient, authorized by host. Fetch help topic access."},
                "access":{"type":["object","null"],"description":"Complete access replacement; owner_reach, grants, denies. Fetch help topic access."},
                "operations":{"type":"array","maxItems":128,"items":{"type":"object","description":"Strict op: create_sheet, put, formula, clear, copy, fill, sort, name. Fetch help topic operations."}},
                "select":selection
            }}),
        )
        .with_provenance(ToolProvenance::new(ToolSourceKind::RustBundle, NAMESPACE)),
    ]
}

fn help(topic: HelpTopic, limits: &Limits) -> Json {
    match topic {
        HelpTopic::Overview => {
            json!({"help":"Use botus_apply create with title, request_id and optional operations. Then use returned document_id and revision for edit/share/transfer/delete. botus_read identity returns the current caller_principal for use as a sharing recipient, never as authentication. list returns accessible documents; describe gives sheets/names/access; read takes id and query. query fields: sheet (Sheet1), range (occupied bounds), columns (letters), view (values/formulas), offset (physical rows), limit (100), filter {column,op,value}. Values are number, Unicode text, boolean or null. Formula cells are {formula:\"=SUM(A1:A9)\"}. Dates are ISO text. Replay returns the original receipt only; reread for current cells. No export, macros or WK1 compatibility. Fetch one complete reference chapter with botus_read {action:\"help\",topic:\"read\"}. Topics: read, apply, operations, formulas, access, limits. Chapter text is in help, except operations and limits use notes. No shell or separate resource reader is required."})
        }
        HelpTopic::Read => {
            json!({"help":include_str!("../skills/botus-1-2-3/references/read.md")})
        }
        HelpTopic::Apply => {
            json!({"help":include_str!("../skills/botus-1-2-3/references/apply.md")})
        }
        HelpTopic::Operations => json!({"operations":[
            {"op":"create_sheet","name":"Budget"}, {"op":"put","at":"A1","values":[["Floppy reserve",3,true,null,{"formula":"=B1*4"}]]},
            {"op":"formula","at":"C2","formula":"=SUM(B2:B9)"},{"op":"clear","range":"A2:B9"},
            {"op":"copy","range":"A1:B3","to":"D1","target_sheet":"Budget"},{"op":"fill","range":"A1:B1","to":"A2:D9"},
            {"op":"sort","range":"A1:C9","keys":[{"column":"B","descending":true}],"header":true},
            {"op":"name","name":"COSTS","range":"B2:B9"}
        ],"notes":include_str!("../skills/botus-1-2-3/references/operations.md")}),
        HelpTopic::Formulas => {
            json!({"help":include_str!("../skills/botus-1-2-3/references/formulas.md")})
        }
        HelpTopic::Access => {
            use documents::{Audience, DeniedOperation, Deny, Grant, Principal, Reach, Role};
            let agent = Principal::Agent("stable-agent-identity".into());
            let audience = Audience::Agent {
                principal: agent.clone(),
                reach: Reach::Forks,
            };
            let worker = Principal::Worker {
                mob_id: "child-reported-mob-id".into(),
                member_id: "child-reported-member-id".into(),
                creation_id: "child-reported-creation-id".into(),
            };
            let share_with_worker = ApplyAction::Share {
                request_id: "share-with-helper".into(),
                id: DocumentId("workbook-id".into()),
                revision: Revision("current-revision".into()),
                access: DocumentAccess {
                    owner_reach: Reach::Forks,
                    grants: vec![Grant {
                        audience: Audience::Agent {
                            principal: worker.clone(),
                            reach: Reach::SelfOnly,
                        },
                        role: Role::Reader,
                    }],
                    denies: vec![],
                },
            };
            json!({"help":include_str!("../skills/botus-1-2-3/references/access.md"),"examples":{
                "owners":[Owner::Agent(agent),Owner::Mob("mob-id".into()),Owner::Realm],
                "audiences":[audience.clone(),Audience::Mob("mob-id".into()),Audience::Realm],
                "worker_identity":{"caller_principal":worker},
                "share_with_worker":share_with_worker,
                "access":DocumentAccess { owner_reach: Reach::Forks, grants: vec![Grant {audience, role:Role::Reader}], denies: vec![Deny {audience:Audience::Agent {principal:Principal::Agent("revoked-agent".into()),reach:Reach::Descendants},operation:DeniedOperation::Read}] }
            }})
        }
        HelpTopic::Limits => {
            json!({"limits":{"sheets":limits.max_sheets,"rows":limits.max_rows,"columns":limits.max_columns,"stored_cells":limits.max_cells,"cell_text_bytes":limits.max_cell_text_bytes,"total_text_bytes":limits.max_total_text_bytes,"document_bytes":limits.max_document_bytes,"formula_bytes":limits.max_formula_bytes,"formula_nodes":limits.max_formula_nodes,"batch_operations":limits.max_batch_operations,"operation_cells":limits.max_operation_cells,"read_rows":limits.max_read_rows,"read_cells":limits.max_read_cells,"scan_rows":limits.max_scan_rows,"evaluation_steps":limits.max_evaluation_steps,"dependency_depth":limits.max_dependency_depth,"response_bytes":limits.max_response_bytes,"request_bytes":MAX_REQUEST_BYTES},"notes":include_str!("../skills/botus-1-2-3/references/limits-and-errors.md")})
        }
    }
}
