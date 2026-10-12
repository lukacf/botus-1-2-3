# botus_apply functional reference

Retrieve this complete chapter without file access using
`botus_read {"action":"help","topic":"apply"}`; the text is in `help`.

`botus_apply` performs one durable mutation on one workbook. Examples show
logical JSON arguments/results, inside any transport's tool-result wrapper.
All calls require a current host-authenticated caller. No argument can supply
caller identity, realm, lineage, filesystem path, or management authority.

Every request requires `action` and `request_id`. `action` is exactly
`create`, `edit`, `share`, `transfer`, or `delete`. Unknown fields and fields
belonging only to another action reject. Optional fields accept `null` only
where explicitly marked nullable. See [limits and errors](limits-and-errors.md).

## Common fields and request identity

| Field | Type | Meaning |
| --- | --- | --- |
| `request_id` | String | Required for every action; unique logical mutation ID, 1-512 UTF-8 bytes, no control characters |
| `id` | String | Required except for `create`; exact opaque document ID |
| `revision` | String | Required except for `create`; exact current opaque revision from a read/description/receipt |

Request IDs are scoped to the host realm, Botus namespace, and actual caller
principal, not only to one workbook. Do not reuse an ID for a different
workbook or action. Forks can have separate caller identities and therefore
separate receipt scopes; a child cannot replay a parent's request by copying
its ID.

Typed defaults and JSON field order are normalized before fingerprinting the
original request. The action, ID, revision, operations, selected query, owner,
access, and other typed fields are part of that fingerprint. Use unchanged
arguments for retries; do not rely on normalization to change a request.

## create

| Field | Type | Required | Default / behavior |
| --- | --- | --- | --- |
| `title` | String | Yes | At most 1024 UTF-8 bytes; empty allowed |
| `operations` | Array of [operations](operations.md) | No | `[]`; a blank workbook is allowed |
| `select` | Nullable [query](read.md#query-fields) object | No | `null`; optional bounded read of the resulting workbook |
| `owner` | Nullable [Owner](access.md#owners) | No | `null`; owner becomes current caller |
| `access` | [Access](access.md#access-object) object | No | Default: `owner_reach` is `forks`, grants/denies are empty; `null` rejects |

A workbook starts with the empty sheet `Sheet1`. Operations run in order.
Requested nondefault ownership must be valid and the caller must be allowed
to create for that owner; it is not impersonation.

```json
{"action":"create","request_id":"budget-create-unique-1","title":"Floppy budget","operations":[{"op":"put","at":"A1","values":[["Item","Quantity","Cost"],["Floppies",3,{"formula":"=B2*2"}]]}],"select":{"range":"A1:C2","limit":2}}
```

For an empty workbook, `{"action":"create","request_id":"blank-unique-1","title":"Notes"}`
is sufficient. Use a fresh ID for each intended new workbook.

## edit

In addition to the common fields:

| Field | Type | Required | Default / behavior |
| --- | --- | --- | --- |
| `operations` | Array of [operations](operations.md) | Yes | Nonempty; at most the host batch limit |
| `select` | Nullable [query](read.md#query-fields) object | No | `null`; optional bounded result selection |

```json
{"action":"edit","request_id":"budget-edit-unique-1","id":"document-id","revision":"current-revision","operations":[{"op":"put","at":"B2","values":[[4]]}],"select":{"range":"B2:C2","limit":1}}
```

The edit preserves title, owner, and access. It first reads the existing
workbook, so read permission and edit permission are both required. There
is no rename-workbook action. Even a successful operation batch with no
changed cells is a mutation and receives a new revision.

## share

Required additional field: `access`, a complete [Access](access.md#access-object)
object. It replaces all access fields, not a patch or appended grant. Defaults
inside that object still apply: omitted grants/denies become empty and
omitted `owner_reach` becomes `forks`.

```json
{"action":"share","request_id":"share-unique-1","id":"document-id","revision":"current-revision","access":{"owner_reach":"descendants","grants":[],"denies":[]}}
```

This permits owner-derived data access through allowed descendant lineage.
Use it only when that audience is intended. Read current access before
replacing it, preserving any grants and denies that should remain. Ownership
administration is required; being an editor is insufficient.

## transfer

| Field | Type | Required | Default / behavior |
| --- | --- | --- | --- |
| `owner` | [Owner](access.md#owners) object | Yes | Host-validated destination; `null` rejects |
| `access` | Nullable [Access](access.md#access-object) object | No | Omitted or `null` preserves current access; an object replaces it |

```json
{"action":"transfer","request_id":"transfer-unique-1","id":"document-id","revision":"current-revision","owner":{"kind":"agent","value":{"kind":"agent","value":"host-validated-recipient"}}}
```

Replace the example recipient with a valid host principal. The caller must
administer the current owner, and the destination must be accepted by the
host. Transfer does not require the caller to become the new owner. It can
remove the caller's ability to read, administer, or replay this mutation.

## delete

Only the common fields are accepted:

```json
{"action":"delete","request_id":"delete-unique-1","id":"document-id","revision":"current-revision"}
```

Requires ownership administration. Deletes workbook content; subsequent
ordinary reads/listings omit it or return `unavailable`. The provider retains
a tombstone and successful receipts for authorized retry handling. There is
no undelete, caller-chosen replacement ID, or worksheet deletion action.

## Response

Every successful response contains `receipt` and `replayed`:

```json
{"receipt":{"document_id":"document-id","revision":"new-opaque-revision","deleted":false},"replayed":false,"changes":{"operations":1,"changed_cells":1,"changed_ranges":[{"sheet":"Sheet1","range":"B2"}],"sheets":1,"total_cells":6},"selected":{"sheet":"Sheet1","range":"B2:C2","columns":["B","C"],"rows":[[4,8]],"row_numbers":[2],"next_offset":null,"scanned_rows":1}}
```

- `receipt.document_id` identifies the workbook; use it as future `id`.
- `receipt.revision` is the revision produced by that committed mutation,
  not a guarantee that no later mutation has occurred.
- `receipt.deleted` is true only for a delete receipt.
- `replayed` says whether this is an earlier successful mutation's receipt.
- `changes` is present for a nonreplayed edit or create with operations.
  It is omitted for blank creation, share, transfer, delete, and all replays.
- `selected` is present only for a nonreplayed create/edit with `select`.
  It uses the [read selection shape](read.md#read-response-and-pagination).
  Optional result fields are omitted, not emitted as `null`.

`changes` always has `operations` (number executed), `changed_cells` (net
stored cell differences), `changed_ranges` (one bounding rectangle per changed
sheet), `sheets` (resulting count), and `total_cells` (resulting nonblank cell
count). A newly created empty sheet has a changed-range entry with `range:
null`. Named-range changes do not count as changed cells. Formula result
changes alone do not count unless their stored formula cell changed.

## Atomicity, conflicts, and retries

The batch is applied to a candidate workbook, all its formulas are checked,
and serialization plus requested selection and response-size checks happen
before commit. A bad late operation, calculation, or oversized selection
rejects the entire mutation. The provider checks current document permission
and expected revision in the same transaction as the write and receipt.
Concurrent cell edits and ACL/ownership mutations can cause a conflict.

- On success, retain the receipt. Do not issue a second request ID merely to
  verify that the first write happened; use `botus_read`.
- If the response is lost or transport completion is uncertain, retry the
  same request ID with unchanged arguments. A committed request replays
  without evaluating or applying operations again.
- Reusing an ID with changed content returns `request_id_reused` when the
  caller still has authority to inspect that receipt.
- On `conflict`, read current state, reconcile, and submit a new request ID
  with the new revision. Replacing only the revision under the old ID is not
  the correct retry procedure.
- A replay returns only the original receipt and `replayed: true`; fresh
  selected cells are deliberately omitted. Read again when current data is
  needed.
- Current authority is required even for a receipt. Revocation, transfer,
  retirement, or lost host authority can deny a retry. A receipt does not
  resurrect deleted content or override current policy.

Successful receipts are retained by the document-provider contract. There is
no user-facing save step, automatic export, WK1 file, or independent Botus
database.
