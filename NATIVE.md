# The filing cabinet belongs to the host

The optional `mobkit` feature supplies exactly two tools: `botus_read` and
`botus_apply`. The default crate uses only Serde and Serde JSON. This package
is unpublished; the native feature depends on reviewed upstream extension
state and runtime authority changes, not a released compatibility promise.

For this unpublished revision, a consuming host must copy the
`[patch.crates-io]` table from [Cargo.toml](Cargo.toml) into its own root
manifest. Cargo does not inherit patches from dependencies. Keep the host
and Botus on the same patched Meerkat version so their runtime types agree.

## Host registration

Register the factory before building the runtime, using the host's configured
durable storage provider. Put `botus-1-2-3` in the intended profiles'
`tools.rust_bundles`. Profiles which do not select the bundle receive no Botus
tool definitions. See [the host composition example](examples/native_host.rs).
It adds Botus to an already configured builder; it does not replace your
identity/provider setup or claim to run a restarted host.

```rust,ignore
use botus_1_2_3::native::BotusToolFactory;
use std::sync::Arc;

let builder = builder.register_tool_bundle_factory(
    "botus-1-2-3",
    BotusToolFactory::requirements(),
    Arc::new(BotusToolFactory::default()),
);
```

The factory refuses a different document namespace. MobKit owns storage,
realm binding, current caller resolution, revision checks, access control and
retry receipts. Botus does not select a path or open a database. Unsupported
or non-durable storage refuses activation. The native document limit is
capped at the host's 8 MiB limit.

The current SDK `GatewayContinuityStore` cannot prove identity binding
history. Registering this bundle with that store returns
`UnifiedRuntimeBuilderError::ExtensionAuthorityUnavailable` before document
storage or tool factory activation. A host needs a continuity provider that
supplies the required authority history.

## Two keys, no function-key overlay

Create with a unique request ID. Ordinary strings are always text; formula
objects are explicit.

```json
{
  "action":"create",
  "request_id":"floppy-budget-1",
  "title":"Emergency floppy budget",
  "operations":[
    {"op":"put","at":"A1","values":[["Item","Units","Price"],["Spare F9 keys",2,12]]},
    {"op":"formula","at":"D2","formula":"=B2*C2"}
  ],
  "select":{"range":"A1:D2"}
}
```

The result contains `receipt.document_id`, an opaque `receipt.revision`,
change counts/ranges and the requested `selected` matrix. Subsequent edit,
share, transfer and delete requests require the document ID and current
revision. Read with:

```json
{"action":"read","id":"document-id-from-receipt","query":{"range":"A1:D2"}}
```

`botus_read` also accepts `list`, `describe` and `help`. Fetch detailed help
on demand with `{"action":"help","topic":"operations"}`. Other topics are
`overview`, `formulas`, `access` and `limits`. The access help examples are
serialized from the actual host types. The concise initial tool definitions
have a 6 KiB combined regression budget.

Use the same request ID and original request to retry a lost response. A
replay returns the exact durable receipt with `replayed:true`, omitting
calculated cells and change statistics. Read again for current cells. A
changed request under the same ID fails. A concurrent duplicate which loses
the commit race also returns receipt-only. Only the actual commit returns
newly calculated cells, so the virtual F9 key cannot print a phantom result.

## Current authority, every time

The adapter resolves the originating session on every call, including help
and retries. Calling a shared dispatcher from a fork does not borrow its
parent's identity. Tool arguments cannot claim an actor, realm, ancestry,
administrator flag or filesystem path. Unknown fields reject.

If the resolver cannot establish current caller authority, the adapter returns
the runtime's existing `ToolError::ExecutionFailed` with the fixed message
`Caller authority unavailable`. This applies to reads, help, edits and retries;
it is infrastructure feedback rather than an access-denied decision. The
adapter does not turn unavailable identity into an anonymous caller.

Owners administer workbooks. Reader and Editor grants do not confer sharing,
transfer or deletion rights. A requested owner is only a recipient for the
host to authorize. Fork and descendant access always depends on the host's
verified creation history and current policy. Missing or unsupported
provenance fails closed; this guide makes no claim for remote placements or
legacy histories which cannot prove it.

An Agent owner uses the host identity authority's durable `AgentIdentity`,
not a display label or a new worker which happens to reuse a name. A worker
owner includes the mob, member and immutable creation ID. Restoring that
same creation preserves its principal; creating a replacement worker with
the same name does not. The host's identity authority must explicitly bind
a durable identity to a successor before that successor can act as it.

Temporary delegate helpers and their forks retire when the delegation
completes. MobKit implicit child mobs currently have in-memory journals.
These agents can share the same durable workbook while active; retiring
them leaves the workbook intact. Restart acceptance for members and forks
of a persistent root mob is tracked separately in the MobKit tests.

Fork access uses the current document ACL and the last persisted effective
native policies of the exact source and executing sessions. The factory
combines profile restrictions with the spawn policy in that effective field;
the separately stored spawn policy is not the permission ceiling. A host
policy change can widen an existing ACL route; creation records do not
freeze or duplicate that policy.
The host reads retained metadata even when a source session is archived.
Missing metadata, unsupported reads or unresolved policy refuse access.
These checks identify the executing agent and observe committed policy;
they do not establish the original human requester or an execution-time
currentness fence. Native tool execution remains independently enforced.

## Limits and evidence

Reads are bounded by rows, cells, evaluation work and actual serialized wire
bytes, including JSON escaping. Edits preflight selected output before
committing. A failed batch or oversized selection leaves the old revision
intact. Reads never persist recalculation caches. Protected exports, whole
workbook dumps, WK1 and macros are deferred.

`tests/native.rs` exercises native dispatch with the real SQLite document
backend and an explicit test authority. Its reopen tests close and reopen
that backend; they do not by themselves prove MobKit host startup or runtime
lineage reconstruction. Full host restart and provenance acceptance require
the separate MobKit runtime tests. Keep those evidence states separate when
shipping a host integration.

The unavailable-authority regression supplies a valid origin session whose
test resolver returns `AuthorityUnavailable`. It checks the fixed infrastructure
error for create, help, read and retry, an empty real SQLite document list after
the failed create, and a successful sibling create through the same dispatcher.
It does not itself cancel or reopen a real native identity. MobKit's separate
cancellation and restart tests establish that lifecycle behavior.

The adapter retry regression also controls this ordering: an identical edit
commits after a missing-receipt lookup, then the owner deletes the document
before the duplicate reads it. The duplicate may recover the original
receipt only if the provider still authorizes it against the tombstone.
Removing the editor's grant before deletion continues to refuse recovery.
