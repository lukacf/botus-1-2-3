# Botus 1-2-3 implementation specification

Status: approved product direction, implementation contract. The MobKit
storage/lineage design is reviewed separately before integration.

## Purpose and packaging

Provide agents without filesystem or shell tools a durable spreadsheet
workspace. Package and repository name: `botus-1-2-3`. Rust crate identifier:
`botus_1_2_3`. Keep the engine independent of Meerkat/MobKit and enable native
integration through optional dependencies. An application which omits the
package gets no tool definitions, prompts or spreadsheet dependency.

Lotus 1-2-3 Release 2.2 inspires the interaction model. Workbooks use
versioned JSON; WK1 and Lotus binary compatibility are not provided.
Tool responses stay concise and bounded.

## Workbook model

- Versioned JSON document with named sheets, sparse cells and named ranges.
- Start with one sheet named Sheet1. A1 cell addresses are case-insensitive
  on input and canonical uppercase on output. Ranges are rectangular.
- Cell values: finite number, Unicode text, boolean, blank or explicit
  formula. Data and formula representations are distinct; an ordinary
  string starting with `=` remains text unless a formula is requested.
- Empty cell, empty string, zero and false remain distinguishable. Missing
  cells read as blank. Text has explicit byte/cell/document bounds and
  exceeds them with errors, never truncation. Dates can initially be ISO
  text; there is no Lotus serial-date compatibility claim.
- Document versions newer than supported are refused before mutation.
  Invalid or oversized stored data is refused, not silently repaired.
- Typed internal representations may be verbose; agent-facing matrix data
  uses JSON primitives and explicit formula objects for compactness.

## Engine operations

Atomic batches support create_sheet, put matrix at an address, formula,
clear range, copy/fill range, sort rows and define named range. Copy/fill
adjust relative formula references while preserving absolute `$` anchors.
Sort has explicit keys and header behavior; define formula movement
semantics and reject unsupported cases rather than silently changing
meaning. Validate a complete batch before replacing the source workbook.

Reads support range selection, formula/value views, column selection,
bounded pagination and simple typed row filters. A value-only result avoids
repeating addresses and metadata for every cell unless requested. Missing
read ranges use actual occupied bounds subject to the same output budget.
Large implicit rectangles must fail or paginate without dense allocation.

## Formula contract

Implement arithmetic (+, -, *, /, ^), parentheses, numeric comparisons,
cell/range references, SUM, AVERAGE, MIN, MAX, COUNT and lazy IF. Support
absolute references and named ranges. Basic lookup is useful if it can be
implemented faithfully; unsupported functions return explicit errors.
Familiar `=SUM(B2:B20)` syntax is canonical. An optional `@SUM` alias must
share the same parser and semantics.

Formula execution is deterministic, bounded and independent of the host:
no shell, network, file reads, volatile time or random calls. Detect cycles,
divide-by-zero, non-finite results and invalid references. IF must evaluate
only the selected branch. Maintain a per-call memo and budget shared across
the whole operation/read, so many requested cells cannot multiply limits.
Do not persist recalculation or caches during a read-only tool call.

Literal text in numeric aggregation is handled by a documented, consistent
rule. Direct scalar arithmetic must not silently coerce arbitrary strings.
Formula errors are represented distinctly from ordinary text values and
must remain visible. Unsupported syntax must never become a zero or blank.

## Two agent tools

`botus_read`: report the current caller principal, list accessible workbooks,
describe a workbook, read a bounded range/query, or load help. `botus_apply`:
create a workbook, atomically edit one, or request an authorized
sharing/ownership operation. All stored writes require the current revision
except creation. Edits may request a small selection of result cells so
recalculation does not require another call.

The `identity` read action accepts only `{"action":"identity"}` and returns
`{"caller_principal": <tagged host Principal>}` from the already-resolved
host access context. Return no realm, policy, ancestry or other host metadata,
and create no scratch document. A parent may use the exact child-reported
Agent or Worker principal as a grant recipient, never as authentication.
Never synthesize Worker identity from names or session IDs. The same caller
resolution, unavailable-authority behavior and output bounds apply.

Tool inputs contain document IDs, data and intended actions. They never
contain authoritative actor identity, realm, mob membership, ancestry,
administrator status or filesystem paths. Unknown fields are rejected.
Agent tools may request a different owner, but only the host storage
service may authorize that request.

Keep the combined serialized tool definitions at or below 6 KiB initially,
with regression coverage. Complex help is fetched on demand. Results are
bounded by cells and serialized bytes. Never return a whole workbook or
base64 export by default. JSON/CSV exports use a protected artifact handle
only if export access is enforced; otherwise provide a bounded explicit
read. Keep revision tokens opaque.

`botus_read` declares ReadOnly. `botus_apply` declares Mutating. One logical
edit batch produces one durable commit and a short receipt with revision,
changed ranges/counts and requested cells. A repeated request ID must not
duplicate writes, while changed content reusing the same ID is rejected.

## MobKit persistence and authority

Botus consumes MobKit's configured, provider-owned extension document
service. It must not open its own independent database, choose ambient
paths or stash workbooks in sessions, identity records or semantic memory.
The host storage layer owns durable documents, namespaces, access metadata
and revision comparisons. Botus owns the workbook payload schema and math.

Each workbook initially persists as one document. Evaluation/serialization
happen before conditional commit; the store rechecks access and revision
atomically with the write. Concurrent edit, ACL change or ownership change
must not be silently overwritten. A failed batch leaves stored bytes and
revision untouched. Storage errors never fall back to memory.

The service binds the package namespace `botus-1-2-3` and host realm.
Ownership may be Agent, Mob or Realm. Agent-private is the creation default
chosen by the user. Ownership controls administrative actions; Reader and
Editor grants do not confer sharing, transfer or deletion rights. Realm
ownership and realm-wide data access are separate facts.

Forks inherit permitted read/edit access to the original workbook, never a
copy and never ownership. A parent can explicitly grant descendant access
for spawned children. The storage design must distinguish fork-only from
broader child delegation, cap inherited permissions, prevent amplification
and enforce revocation on subsequent calls. Host-derived lineage survives
restart. Claims in tool arguments, console labels or transcript content
are never lineage evidence. Fork dispatchers may be shared Arcs, so resolve
the actual caller from dispatch context every time.

Persistence does not change native member lifetime. Temporary delegate
helpers and their forks retire when delegation completes; their disappearance
does not delete the workbook. MobKit's implicit child mobs currently have
in-memory journals. Restart lineage guarantees apply to persistent mobs and
members. Do not claim that a durable workbook makes an ephemeral child mob
durable, or add a private Botus lineage archive to simulate that guarantee.

Restoring a session or respawning a stable agent must not change workbook
ownership. Deleting an agent/session must not silently delete its workbook.
Only authorized document deletion or a defined retention process may do so.

## Initial boundaries

Required delivery: tested pure engine, compact tools exposed through a real
MCP server, an optional native adapter, MobKit-backed durable storage,
owner/sharing/lineage behavior, an installable package with a self-contained
skill and full read/apply references, example host, and
restart/concurrency/access tests. A static golden file or fake memory store
is not evidence for provider integration. See [MCP-DESIGN.md](MCP-DESIGN.md)
for the caller-context and transport contract.

Deferred explicitly: GUI, printer emulation, Lotus macro language, WK1
fidelity, Excel compatibility, arbitrary scripts and advanced charts.
An independent Botus database is not part of the MCP deployment: MobKit
continues to own persistence and authority.

## Acceptance evidence

- Engine: empty/text/number distinctions, Unicode, sparse distant cells,
  malformed ranges, formula chains, IF laziness, cycles, named ranges,
  copy anchors, sorted records and atomic rejection of a late failing op.
- Bounds: sparse reads, huge range refusal/pagination, expression depth,
  evaluated-cell/work budget, document size and actual response bytes.
- Integration: create/edit/read through native dispatch; close all storage
  handles, restart the MobKit host and recover the same workbook and ACL.
- Access: another agent cannot list, read, change, share or delete private
  data; Reader cannot edit; Editor cannot administer; unauthorized and
  missing IDs have equivalent externally visible errors.
- Identity: parent/child/parent reads through the same shared dispatcher
  return only each call's resolved principal without creating documents;
  unknown caller-authority input fields reject.
- Lineage: fork_off in a persistent root mob uses the original workbook after
  restart; temporary cross-mob helpers and their forks share during their
  native lifetime and the workbook survives their teardown. Ordinary
  children require the allowed audience, delegated access cannot amplify,
  and revoking the source grant removes inherited access.
- Concurrency: two editors of one revision yield one winner, ACL revocation
  races cannot admit stale writes, and retry IDs preserve exactly one
  committed result. Delete/recreate cannot resurrect stale authority.
- Isolation: realm, namespace and stable-identity scoping; untrusted input
  cannot manufacture actor, owner-management or lineage claims.
- Packaging: default core build lacks runtime dependencies, adapter feature
  compiles and runs its tests, a real MCP client exercises the installed
  entry point, all skill reference assets ship in the package, and all
  commands use the repository wrapper.
- Review: adversarial findings are fixed or explicitly recorded as release
  blockers. Local validation, PR CI, merge and package publication are
  separate evidence states.
