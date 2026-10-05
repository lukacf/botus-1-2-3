# Workbook operation reference

These are the eight accepted objects in a create/edit action's `operations`
array. Each requires `op`; unknown fields reject. Operations run in order
inside one atomic mutation. Read [apply](apply.md) before submitting a batch.

Except `create_sheet`, every operation accepts optional string `sheet`,
defaulting to `"Sheet1"`; `null` rejects. The sheet must exist when that
operation runs. Sheet lookup ignores case and preserves its stored name.

## Values and coordinates

Cell inputs are finite JSON numbers, strings, booleans, `null`, or a formula
object with exactly the string field `formula`:

```json
{"op":"put","at":"A1","values":[[null,"",0,false,"=1+2",{"formula":"=1+2"}]]}
```

These six cells mean blank, empty text, number zero, boolean false, literal
text, and a formula evaluating to three. Missing cells are blank. Writing
`null` removes a stored cell; empty text remains stored. Arbitrary objects,
arrays as individual cells, and nonfinite numbers are unsupported. There
are no date, currency, formatting, hyperlink, or binary cell types. Use ISO
text for dates. See [formulas](formulas.md) for calculated values.

Numbers use IEEE 754 binary `f64` precision. Finite integers beyond `2^53`
may round, and decimal fractions are not generally exact. Store exact IDs
as text; do not assume exact decimal currency arithmetic.

- Addresses are ASCII column letters plus a positive one-based row: `A1`,
  `aa12`. Inputs are case-insensitive and canonical output is uppercase.
  Leading row zeros normalize (`A01` becomes `A1`). No spaces, `$` anchors,
  sheet prefixes, or named ranges are allowed in operation addresses.
- Ranges are one address, `A1:C9`, or the alias `A1..C9`. Endpoints are
  inclusive and must run from top left to bottom right. A single-cell range
  is valid; reversed, whole-column (`A:A`), whole-row, or disjoint ranges
  are not. Canonical ranges use `:` and collapse a single cell to `A1`.
- `$` anchors and sheet-qualified references belong inside formulas, not in
  operation/query range strings.
- Coordinates must fit the host's row/column limits. Address strings are
  limited to 64 bytes; range strings to 128 bytes.

## create_sheet

Required fields: `op: "create_sheet"`, `name` string. No `sheet` field.

```json
{"op":"create_sheet","name":"Budget 1989"}
```

Creates an empty sheet without changing `Sheet1`. Names must be nonempty,
at most 128 UTF-8 bytes, without leading/trailing whitespace, control
characters, or any of `[]:*?/\!`. Internal spaces and apostrophes are
allowed. Names must be unique ignoring case. Recreating an existing sheet
is an error, not an idempotent no-op. The host sheet limit applies.

## put

Required fields: `op: "put"`, `at` address string, `values` matrix.
Optional: `sheet`.

```json
{"op":"put","sheet":"Sheet1","at":"A1","values":[["Quarter","Cost"],["Q1",24],["Q2",16]]}
```

Writes the matrix with its top-left cell at `at`, overwriting existing cells
in the destination rectangle. It must have at least one row and column, and
every row must have identical width. It must fit the grid and operation
budget. `null` entries clear corresponding cells; cells outside the rectangle
are unchanged. Formulas are explicit `formula` objects inside the matrix.

## formula

Required fields: `op: "formula"`, `at` address string, `formula` string.
Optional: `sheet`.

```json
{"op":"formula","at":"B4","formula":"=SUM(B2:B3)"}
```

Writes one explicit formula cell. A leading `=` is conventional but optional
inside this field. Unsupported syntax/functions and exceeded formula limits
reject. A formula must be valid under the final batch's calculation, including
references to cells or names introduced by later operations.

## clear

Required fields: `op: "clear"`, `range` string. Optional: `sheet`.

```json
{"op":"clear","range":"A2:B9"}
```

Removes stored cells inside the rectangle. It does not delete rows, shift
cells, delete the sheet, or remove named ranges. Existing formulas elsewhere
continue referencing these now-blank addresses. Clearing an empty region
can succeed with zero changed cells. The work budget accounts for scanning
the sheet's stored cells, not the rectangle's potentially huge blank area.

## copy

Required fields: `op: "copy"`, `range` source rectangle, `to` destination
top-left address. Optional: `sheet`; nullable string `target_sheet`, whose
omission or `null` means the same sheet as the source.

```json
{"op":"copy","sheet":"Sheet1","range":"A1:B3","to":"D1","target_sheet":"Budget 1989"}
```

The target sheet must already exist. Destination dimensions equal the source
rectangle. Copy includes blanks, which clear overwritten destination cells.
The original source is snapshotted so overlapping copies are stable.

Relative formula references shift by the displacement from each original
cell to its destination; `$` anchors keep the anchored coordinate fixed.
Explicit sheet names remain explicit, but their relative coordinates still
shift. Unqualified references in a cross-sheet copy resolve on the target
sheet. Named-range references retain their name and existing definition.
Copied formulas may be rewritten to canonical equivalent syntax. A shift
outside the grid rejects the batch. Copy does not copy or retarget name
definitions or separately clear the source. When source and destination
overlap, destination writes overwrite the intersecting source cells; copied
values still come from the original source snapshot.

## fill

Required fields: `op: "fill"`, `range` source rectangle, `to` destination
rectangle. Optional: `sheet`. There is no `target_sheet` field.

```json
{"op":"fill","range":"A1:B1","to":"A2:F5"}
```

Tiles the source across the destination, in this example three tiles across
and four down. Both destination dimensions must be whole multiples of the
source dimensions. Source and destination are on the same sheet. Blank
copying, overlap snapshots, reference shifting, and errors follow `copy`.
Each formula shifts relative to its corresponding source cell, including
across successive tiles. This repeats a pattern; it does not extrapolate a
numeric/date sequence.

## sort

Required fields: `op: "sort"`, `range` rectangle, `keys` array of 1-16 sort
keys. Optional: `sheet`; boolean `header` default `false`.

Each key is exactly `{column, descending?}`: `column` is absolute column
letters inside the selected rectangle; `descending` is a boolean, default
`false`. These are sheet coordinates, not field names or offsets.

```json
{"op":"sort","range":"A1:C20","keys":[{"column":"B","descending":true},{"column":"A"}],"header":true}
```

Moves entire rows within the selected rectangle only. Outside columns do not
move. `header: true` keeps the rectangle's first row fixed. Key order sets
priority; ties preserve original row order. Duplicate keys are accepted but
usually unnecessary.

Ascending type order is blank, boolean, number, text; booleans order
`false < true`, finite numbers use total numeric order, and strings compare
case-sensitively in string order. Descending reverses that key's ordering,
including type rank. Formula cells anywhere in the sorted data rows reject,
even outside key columns. A preserved header may contain formulas. Formulas
outside the sorted data retain their addresses and recalculate against the
new values. Named ranges retain their rectangles.

## name

Required fields: `op: "name"`, `name` identifier, `range` rectangle.
Optional: `sheet`.

```json
{"op":"name","name":"DISK_COSTS","sheet":"Sheet1","range":"B2:B9"}
```

Creates or replaces a workbook-wide named range. Names are uppercased before
validation and looked up without case sensitivity. The resulting name is at
most 128 bytes, starts with an ASCII letter or `_`, and then contains only
ASCII letters, digits, or `_`. It cannot be a cell address or `TRUE`/`FALSE`. The referenced sheet
must exist. There is no delete-name operation; redefining a name replaces its
sheet/range. The count of names may not exceed host `stored_cells`.

An aggregate can use a named range directly, for example `=SUM(DISK_COSTS)`.
A scalar use is allowed only for a single-cell name. No operation/query
`range` field accepts a name in place of an A1 rectangle.

## Batch behavior and unsupported operations

All eight operations participate in [apply's atomicity and receipt rules](apply.md).
Intermediate writes are checked against cell/text budgets; a later clear
does not excuse an earlier overflow. After all operations, every stored
formula is evaluated under one shared calculation budget. The optional
selected result is a separate bounded read of that candidate.

There are no insert/delete row or column, move, transpose, rename/delete
sheet, rename workbook, import/export, formatting, chart, macro, or arbitrary
script operations in this version. The only `delete` is the top-level
workbook action. Unknown operation names reject rather than being ignored.
