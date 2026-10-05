# botus_read functional reference

Retrieve this complete chapter without file access using
`botus_read {"action":"help","topic":"read"}`; the text is in `help`.

Use the actual discovered tool name. Examples show JSON arguments and logical
JSON results; the transport may wrap the result as tool content. All actions
are read-only and require a host-resolved current caller, even `help` and
`identity`. Reads do not persist calculation caches.

Every call requires `action`, exactly one of `identity`, `list`, `describe`,
`read`, or `help`. Fields from a different action and unknown fields reject.
Optional fields accept `null` only when explicitly marked nullable below.
See [limits and errors](limits-and-errors.md).

## identity

No other fields are accepted:

```json
{"action":"identity"}
```

Response has exactly `caller_principal`, containing the host's current
[Principal](access.md#principals). For example:

```json
{"caller_principal":{"kind":"agent","value":"host-issued-agent-identity"}}
```

A child can report this object for use as a grant recipient. It is not a
credential, ownership grant, or proof supplied back to the tool. No realm,
policy, lineage, or session metadata is returned; no workbook is created.

## list

| Field | Type | Required | Default / meaning |
| --- | --- | --- | --- |
| `limit` | Integer | No | `50`; must be `1-100` |
| `after` | Nullable string | No | `null`; resume after an earlier page's `next` document ID |

```json
{"action":"list","limit":20}
```

Only currently readable, nondeleted documents in this host's Botus namespace
and realm are listed. The page contains summaries, not cells or access rules:

```json
{"documents":[{"id":"document-id","revision":"opaque-revision","title":"Floppy budget","content_type":"application/vnd.botus.workbook+json","schema_version":1,"created_at_ms":0,"updated_at_ms":0}],"next":null}
```

All shown summary fields are present. Timestamps are Unix milliseconds; the
zeros illustrate their type. `next` is a document ID or `null`. Pass a nonnull
`next` unchanged as `after` to continue. The current store orders by document
ID, not title or creation time.

The cursor document must still exist and be readable. If it was deleted or
access was revoked, pagination can return `unavailable`; restart the listing
and reconcile IDs already seen. A page is internally consistent, but pages
do not freeze one snapshot across multiple calls.

## describe

Required field: `id`, the document ID string. No query fields are accepted.

```json
{"action":"describe","id":"document-id"}
```

Response:

```json
{"id":"document-id","revision":"opaque-revision","title":"Floppy budget","owner":{"kind":"agent","value":{"kind":"agent","value":"host-issued-agent-identity"}},"access":{"owner_reach":"forks","grants":[],"denies":[]},"workbook":{"schema_version":1,"sheets":[{"name":"Sheet1","occupied_range":"A1:C2","cells":6,"formulas":1}],"names":{"COSTS":{"sheet":"Sheet1","range":"C2"}}}}
```

- `owner` and `access` use the complete shapes in [access](access.md).
- Each sheet has `name`, nullable `occupied_range`, stored nonblank `cells`,
  and `formulas` count. An empty sheet has `occupied_range: null` and zeros.
- `names` maps uppercase named-range identifiers to `{sheet,range}` objects.
  It is `{}` when none exist. Sheets are listed in stored name order, not
  insertion order.
- Description validates stored workbook structure and formula syntax but
  does not calculate every formula. Unsupported stored format/version fails.

## read

| Field | Type | Required | Default / meaning |
| --- | --- | --- | --- |
| `id` | String | Yes | Document ID |
| `query` | Object | No | `{}` with the defaults below; `null` rejects |

### Query fields

The same query object is used for an apply action's `select`.

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `sheet` | String | `"Sheet1"` | Existing sheet, matched ignoring case |
| `range` | Nullable string | `null` | Inclusive rectangle; omission uses occupied bounds |
| `columns` | Nullable array of strings | `null` | Column letters in desired output order; omission uses all columns in the range |
| `view` | String | `"values"` | `values` calculates cells; `formulas` returns stored values and formula objects |
| `offset` | Nonnegative integer | `0` | Physical rows skipped from the range's top row |
| `limit` | Positive integer | `100` | Maximum returned rows, at most host `read_rows` |
| `filter` | Nullable object | `null` | One row predicate, described below |

Ranges accept `A1`, `A1:C20`, or `A1..C20`; see
[coordinate rules](operations.md#values-and-coordinates). They are grid
rectangles, not named-range expressions. A sheet is selected separately.
Columns are case-insensitive ASCII letters such as `A` or `AA`, not numeric
indices or header labels. An explicit columns array must be nonempty,
unique ignoring case, and entirely inside the range. Its order is preserved.

`offset` must not exceed the range height. Equality returns an empty completed
page. With no explicit range and a completely empty sheet, the result has
`range: null`, empty arrays, and no continuation; there is no rectangle to
scan, so column/filter selection and offset are not evaluated beyond argument
parsing. With an explicit range on an empty sheet, blank rows are returned.

```json
{"action":"read","id":"document-id","query":{"sheet":"Sheet1","range":"A1:D100","columns":["D","A"],"offset":1,"limit":20,"view":"values","filter":{"column":"B","op":"ge","value":10}}}
```

### Filter fields and comparison rules

Every filter requires all three fields, with no extras:

| Field | Type | Meaning |
| --- | --- | --- |
| `column` | String | Absolute column letters inside the query range; need not appear in output columns |
| `op` | String | Exactly `eq`, `ne`, `lt`, `le`, `gt`, `ge`, or `contains` |
| `value` | Primitive | Finite number, string, boolean, or `null`; formula/error objects reject |

The filter always uses the calculated cell value, including under
`view: "formulas"`. `contains` requires a string needle and matches only text
cells with that case-sensitive substring. Other comparisons match like
types only: numbers numerically, text case-sensitively in string order,
booleans `false < true`, and blank equal to blank. Across different types,
`eq`, `lt`, `le`, `gt`, and `ge` are false; `ne` is true. No string-to-number
or blank-to-zero coercion occurs in filters. A formula error in the filter
column fails the read rather than becoming an unmatched row.

### Read response and pagination

```json
{"id":"document-id","revision":"opaque-revision","selected":{"sheet":"Sheet1","range":"A1:C4","columns":["A","C"],"rows":[["Item","Cost"],["Floppies",6]],"row_numbers":[1,2],"next_offset":2,"scanned_rows":2}}
```

`selected` always contains:

- `sheet`: actual stored sheet name.
- `range`: canonical uppercase rectangle, or `null` for implicit empty bounds.
- `columns`: selected uppercase column letters, in output order.
- `rows`: matrix of primitives, [cell errors](limits-and-errors.md#cell-errors),
  or explicit formula objects under formula view. Missing cells are `null`.
- `row_numbers`: absolute one-based sheet rows corresponding to the matrix.
- `next_offset`: nullable physical offset for continuation within this range.
- `scanned_rows`: physical rows examined in this call, including nonmatches.

Set the next call's `query.offset` to `selected.next_offset`, keeping the
other query fields unchanged. Do not add `limit` to the previous offset or
count matched rows. A page may be short or empty and still have continuation
when the scan budget is reached. The cell budget can also lower the effective
page size to `floor(read_cells / selected_column_count)`. `next_offset: null`
marks completion of that range as observed by this call.

Each call returns its own document revision. If consistent multi-page data
is required and the revision changes, restart or reconcile explicitly.
Sparse occupied bounds can be huge; choose explicit ranges and columns.
Output overflow rejects the call without silently truncating cell values.

## help

Optional field `topic` is one of the following; omission means `overview`.
`null` and unknown topic names reject.

| Topic | Response fields | Contents |
| --- | --- | --- |
| `overview` | `help` string | Basic action/query usage and boundaries |
| `read` | `help` string | This complete read reference |
| `apply` | `help` string | Complete apply reference, including retry and receipt rules |
| `operations` | `operations` array, `notes` string | Full operation reference in `notes`, plus examples for all eight operations |
| `formulas` | `help` string | Complete language and evaluation reference |
| `access` | `help` string, `examples` object | Complete access reference plus tagged owner/audience and sharing examples |
| `limits` | `limits` object, `notes` string | Complete limits/error reference in `notes`, plus actual host-selected numeric bounds |

Every non-overview topic returns its entire bundled Markdown chapter, embedded
from the same file shipped with the skill. No shell or separate resource
reader is needed. Follow a relative reference link by requesting its matching
topic above; `limits-and-errors.md` corresponds to `limits`.

Help is subject to the same current caller check and serialized response cap
as other reads. It has no pagination and does not truncate. If a complete
chapter and its tool envelope exceed the cap, the tool returns `limit`; the
host must allow a larger response. If even the error envelope cannot fit,
native dispatch fails with `Botus response exceeds byte limit`; MCP reports a
generic dispatch failure as described in [limits and errors](limits-and-errors.md).

`access.examples` has `owners`, `audiences`, `worker_identity`,
`share_with_worker`, and `access`. Example identities are illustrative
recipients, never real caller credentials. The complete numeric keys returned
by limits help are listed in [limits and errors](limits-and-errors.md).
