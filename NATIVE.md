# The filing cabinet belongs to the host

The optional `meta` feature supplies exactly two tools: `botus_read` and
`botus_apply`. The default crate uses only Serde and Serde JSON. This package
is not published to a package registry. Native integration uses the published
Meerkat `=0.9.0` and `meerkat-meta =0.9.0` crates with `extension-state`.

Use the same native and Meta versions as
[Cargo.toml](Cargo.toml) in the consuming host so their runtime types agree.
The coordinated `Cargo.lock` must resolve that dependency set. The consuming
application does not need a native Git patch table.

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

The factory refuses a different document namespace. Meta owns storage,
realm binding, current caller resolution, revision checks, access control and
retry receipts. Botus does not select a path or open a database. Unsupported
or non-durable storage refuses activation. The native document limit is
capped at the host's 8 MiB limit.

Native identity and lineage survive host restart only when the host also
persists its native mob journal. The convenience `.persistent_state(...)`
path alone leaves that journal in memory. Build `MobBootstrapSpec::persistent(...)`
with storage from `mob_composition_manifest::persistent_mob_storage(...)`,
pass its paired provenance to `.with_mob_storage_provenance(...)`, and supply
the spec with `.mob_spec(...)`. Durable workbooks and continuity records
cannot reconstruct missing native creation facts; unavailable authority still
refuses access. The host owns this storage configuration, and Botus continues
using the host's document service.

Both definitions carry `RustBundle` provenance with source ID `botus-1-2-3`,
so native `InheritParent` delegation can carry the registered bundle's
visibility proof. Runtime visibility and authority gates remain in force.

Explicit bundle activation with `LocalContinuityStore` enables retained
identity history in the owning continuity schema v3. Its backfill, retention
triggers and version stamp commit together. Earlier binaries which support
only continuity schemas v1/v2 refuse the database afterward. For a database
that has not opted in, ordinary open and head-only migration remain on
v1/v2. Current binaries recognize and preserve an existing v3 database even
when the extension is disabled; head-only writes never downgrade it.

The current SDK `GatewayContinuityStore` cannot prove identity binding
history. Registering this bundle with that store returns
`UnifiedRuntimeBuilderError::ExtensionAuthorityUnavailable` before document
storage or tool factory activation. A host needs a continuity provider that
supplies the required authority history.

## Create, read and edit

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

`botus_read` also accepts `identity`, `list`, `describe` and `help`. Fetch a
complete reference chapter without shell access with
`botus_read {"action":"help","topic":"read"}` or
`botus_read {"action":"help","topic":"apply"}`. Other chapter topics are
`operations`, `formulas`, `access`, and `limits`; `overview` is a compact index.
Chapter text is in `help`, except `operations` and `limits` use `notes`. Access
examples still serialize the actual host types, and `limits` reports the
current host configuration. Chapters are embedded from the Markdown files
bundled with the skill. Help needs the same authenticated caller and response
budget as other reads; an oversized complete chapter fails with `limit`,
without truncation. The concise initial tool definitions have a 6 KiB combined
regression budget.

Use the same request ID and original request to retry a lost response. A
replay returns the exact durable receipt with `replayed:true`, omitting
calculated cells and change statistics. Read again for current cells. A
changed request under the same ID fails. A concurrent duplicate which loses
the commit race also returns receipt-only. Only the actual commit returns
newly calculated cells.

## Share with a child's reported principal

A child can call `botus_read` with `{"action":"identity"}` and report the
returned `caller_principal` to its parent. This action accepts no other
fields and returns only the host-resolved tagged principal, subject to the
same response byte bound as other reads. It does not create a workbook.
A Worker principal response has this shape:

```json
{
  "caller_principal":{
    "kind":"worker",
    "value":{
      "mob_id":"child-reported-mob-id",
      "member_id":"child-reported-member-id",
      "creation_id":"child-reported-creation-id"
    }
  }
}
```

Copy the entire reported principal into the grant recipient. Never build a
Worker principal from a display name, session ID or guessed creation ID.
The following `botus_apply` request grants only that worker Reader access:

```json
{
  "action":"share",
  "request_id":"share-with-helper",
  "id":"workbook-id",
  "revision":"current-revision",
  "access":{
    "owner_reach":"forks",
    "grants":[{
      "audience":{
        "kind":"agent",
        "value":{
          "principal":{
            "kind":"worker",
            "value":{
              "mob_id":"child-reported-mob-id",
              "member_id":"child-reported-member-id",
              "creation_id":"child-reported-creation-id"
            }
          },
          "reach":"self_only"
        }
      },
      "role":"reader"
    }],
    "denies":[]
  }
}
```

Sharing replaces the complete access policy, so retain any existing grants
and denies you still need. A reported principal selects a recipient; it
never authenticates the caller or confers management authority. Every call
still resolves its own actor from host dispatch context.

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
transfer or deletion rights. Naming a sheet `Management` does not change
these permissions. A requested owner is only a recipient for the
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
completes. These agents can share the same durable workbook while active;
retiring them leaves the workbook intact. Restart lineage requires the
host's retained native mob journal and creation history. Durable workbook
storage alone cannot restore a missing agent or prove its authority.

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
that backend; they do not by themselves prove Meta host startup or runtime
lineage reconstruction. Full host restart and provenance acceptance require
the separate Meta runtime tests. Keep those evidence states separate when
shipping a host integration.

The unavailable-authority regression supplies a valid origin session whose
test resolver returns `AuthorityUnavailable`. It checks the fixed infrastructure
error for create, identity, help, read and retry, an empty real SQLite document
list after the failed create, and a successful sibling create through the
same dispatcher.
It does not itself cancel or reopen a real native identity. Meta's separate
cancellation and restart tests establish that lifecycle behavior.

The identity regression uses a parent and a Worker child from that explicit
test authority. Parent/child/parent calls through one shared dispatcher must
return their distinct current principals without creating any documents.
It covers maximal escaped Worker identifiers, the serialized response bound
and strict rejection of caller-authority input fields. This fixture does not
replace Meta's runtime provenance tests.

The adapter retry regression also controls this ordering: an identical edit
commits after a missing-receipt lookup, then the owner deletes the document
before the duplicate reads it. The duplicate may recover the original
receipt only if the provider still authorizes it against the tombstone.
Removing the editor's grant before deletion continues to refuse recovery.
