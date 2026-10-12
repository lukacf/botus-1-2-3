# Limits and errors reference

Use `botus_read {"action":"help","topic":"limits"}` to retrieve this complete
chapter in `notes` and the actual host configuration in `limits`, without file
access. Defaults below describe the standard native adapter; the host
can choose lower or different engine limits. The initial tool schemas remain
compact; a schema description is not permission to exceed runtime bounds.

## Configured limits

The `limits` help response contains every key below as an integer, alongside
a `notes` string. Byte counts are UTF-8 or serialized JSON bytes, not
characters. `KiB`/`MiB` here mean powers of 1024.

| Help key | Standard native default | Meaning |
| --- | --- | --- |
| `sheets` | 32 | Maximum workbook sheets |
| `rows` | 100000 | Maximum one-based row number |
| `columns` | 1024 | Maximum column number; column 1024 is `AMJ` |
| `stored_cells` | 100000 | Total nonblank stored cells across sheets; also maximum count of named ranges |
| `cell_text_bytes` | 1048576 | Maximum literal or calculated cell text, and filter text (1 MiB) |
| `total_text_bytes` | 8388608 | Workbook text plus formula-source bytes; also evaluator calculated-text work budget (8 MiB) |
| `document_bytes` | 8388608 | Maximum serialized workbook payload (8 MiB) |
| `formula_bytes` | 16384 | Maximum one formula's source bytes (16 KiB) |
| `formula_nodes` | 512 | Maximum parsed formula nodes |
| `batch_operations` | 128 | Maximum operations in one create/edit batch |
| `operation_cells` | 100000 | Shared mutation work budget across a batch |
| `read_rows` | 1000 | Maximum requested query `limit` |
| `read_cells` | 10000 | Maximum output cells per page |
| `scan_rows` | 10000 | Maximum physical rows scanned per page, including filter nonmatches |
| `evaluation_steps` | 250000 | Shared cell/expression/aggregate work budget per evaluator |
| `dependency_depth` | 64 | Maximum active cell dependency depth |
| `response_bytes` | 262144 | Serialized response cap, including tool-result escaping (256 KiB) |
| `request_bytes` | 8388608 | Maximum raw request and normalized mutation request bytes (8 MiB) |

The pure engine's document default is 16 MiB, but this adapter caps it to the
document provider's 8 MiB ceiling. JSON overhead means an 8 MiB text budget
does not guarantee an 8 MiB payload fits. The host cannot activate the native
adapter with a response budget below 16384 bytes.

The MCP host reserves another 256 bytes for its result envelope before
mutation preflight. With default configuration, its reported `response_bytes`
is 261888; the minimum configured MCP response budget is 16640 bytes. Use the
live help response for the selected host's effective limit.

Full help chapters use the same response bound as other reads, including JSON
escaping and tool envelopes. They are never truncated and have no pagination.
If the whole chapter cannot fit, the tool returns `limit` and the host must
allow a larger response. If even the error envelope cannot fit, native
dispatch fails with `Botus response exceeds byte limit`; MCP reports the
generic dispatch failure described in the tool-error section below.

## Fixed bounds and defaults

| Item | Bound or default |
| --- | --- |
| List page | Default 50; accepted `limit` is 1-100 |
| Read query | Default `limit` 100, `offset` 0, `sheet` `Sheet1`, `view` `values` |
| Request ID / principal identifier | 1-512 UTF-8 bytes, no control characters |
| Provider-issued document ID / revision | Opaque strings, bounded to 512 UTF-8 bytes |
| Workbook title | Up to 1024 UTF-8 bytes; empty allowed |
| Sheet name | 1-128 UTF-8 bytes, with restrictions in [operations](operations.md#create_sheet) |
| Named-range name | 1-128 bytes, identifier restrictions in [operations](operations.md#name) |
| Operation/query address | Up to 64 bytes |
| Operation/query range | Up to 128 bytes |
| Sort keys | 1-16 |
| Access entries | At most 256 grants and denies combined |
| Formula parser nesting and AST depth | At most 64 levels each |
| Formula tokens | At most `4 * formula_nodes + 8` |
| Human-readable tool error message | At most 512 characters |

Mutation cell work is not just net changed cells. `put` charges the whole
matrix, `formula` one cell, `copy`/`fill` the destination area, `sort` the
selected rectangle, and `clear` a scan of its sheet's stored cells. Work is
shared across operations. The formula validation pass shares its evaluator
across all workbook formulas. A read shares its evaluator across selected
cells and filter values. An apply action's optional `select` is a separate
bounded read of the candidate; do not assume a read budget applies anew to
each requested cell.

Reads paginate for row/cell/scan bounds. Oversized individual text or complete
serialized results reject instead of silently truncating data. A read page
may be short or empty with a nonnull continuation. Repeated calculated text
can consume an evaluator's text work budget even when a memoized value is
reused. Mutations validate their requested output before the durable commit.

## Tool errors

Operation failures use a logical error body such as:

```json
{"error":{"code":"conflict","message":"Document revision changed; read it and submit a new request ID"}}
```

The tool result is marked as an error. Codes emitted by the adapter are:

| Code | Meaning / response |
| --- | --- |
| `invalid_input` | Invalid action/field/type, malformed arguments, invalid list limit or document request; correct the input |
| `unavailable` | Missing, deleted, wrong-scope, or unauthorized document/cursor/receipt; these cases are deliberately not distinguished |
| `conflict` | Expected document revision changed; reread, reconcile, and submit a new request ID |
| `request_id_reused` | Same caller's request ID already committed different normalized content; do not alter an existing logical request |
| `workbook` | Engine validation, syntax, reference, calculation, or engine-limit failure; inspect the message and correct/narrow the operation |
| `version` | Stored document content type or schema version is unsupported |
| `limit` | Adapter request/serialized-output cap; narrow a data request/selection or raise the host response cap for a complete help chapter |
| `storage` | Host document storage failed; no fallback to an independent database or memory |
| `serialization` | Adapter could not serialize the result |

Engine errors at the tool boundary are grouped under `workbook`; do not
expect their internal code to become the top-level tool code. A workbook
payload schema mismatch can also be reported through this engine path.

In the native adapter, missing or invalid current caller authority is a
dispatch failure with `Caller authority unavailable`, rather than an ordinary
document error body. If even an error response cannot fit, native dispatch
fails with `Botus response exceeds byte limit`.

The MCP path deliberately returns generic provider or protocol failures,
such as `Botus call authority unavailable` for an invalid correlation and
`Botus caller authority or host service unavailable` for a dispatch failure.
Do not depend on native error text reaching an MCP client. None of these
failures is a workbook save receipt; repair the host binding/configuration
through the host.

Storage/transport failures can leave completion uncertain. Follow
[exact retry rules](apply.md#atomicity-conflicts-and-retries) rather than
assuming the write either committed or did not commit from an error alone.

## Cell errors

In `view: "values"`, a selected formula cell's nonlimit evaluation error is
a distinct cell value, not an ordinary string:

```json
{"error":{"code":"divide_by_zero","message":"Division by zero"}}
```

The internal engine error-code vocabulary is:

| Code | Meaning |
| --- | --- |
| `invalid_input` | Invalid workbook structure, value, name, or operation |
| `invalid_reference` | Invalid/missing sheet, name, coordinate, or shifted reference |
| `formula_syntax` | Malformed expression or invalid function arity |
| `unsupported_function` | Function not in the supported set |
| `formula_type` | Unsupported coercion or scalar/range use |
| `divide_by_zero` | Division by zero or average with no numeric inputs |
| `cycle` | Evaluated circular cell dependency |
| `non_finite` | Nonfinite literal, arithmetic result, or aggregate intermediate |
| `limit` | Engine work, text, grid, size, depth, or count limit |
| `version` | Unsupported workbook payload schema |
| `serialization` | Invalid stored workbook serialization |

Not every code can reach a cell: structural validation can fail before cell
evaluation. `limit` errors fail the entire read, and any formula error in a
filter fails the entire read. Formula view returns formula objects; it does
not turn them into cell errors, although structural validation and filter
evaluation still run. New edit batches reject calculation errors atomically.
