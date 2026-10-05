# The little green spreadsheet manual

The screen is optional. The cells are not. This guide describes the pure
engine; the native tool adapter adds authenticated, revisioned persistence.

## Entering the numbers department

Start with `Workbook::new()` and `Limits::default()`. A workbook starts with
`Sheet1`. Deserialize an array of operations into `Vec<Operation>` and pass
it to `Workbook::apply`. For example:

```json
[
  {"op":"put","at":"A1","values":[
    ["Quarter","Floppy disks","Cost"],
    ["Q1",12,24],
    ["Q2",8,16]
  ]},
  {"op":"formula","at":"C4","formula":"=SUM(C2:C3)"},
  {"op":"name","name":"DISK_BUDGET","range":"C2:C3"}
]
```

Read `A1:C4` with a `ReadRequest`. Value view returns a compact matrix;
formula view preserves explicit formula objects. Ordinary strings beginning
with `=` stay ordinary strings. Data should not accidentally get promoted
to middle management.

Empty cells, empty strings, zero and false are different values. Writing
`null` clears a cell. There is no special Lotus date serial or automatic
conversion of a numeric-looking string. Keep dates as ISO text for now.

## Recalculation without the beige box

Arithmetic, comparisons, references, named ranges, `SUM`, `AVERAGE`, `MIN`,
`MAX`, `COUNT` and lazy `IF` are supported. The optional `@SUM(...)` spelling
uses the same implementation as `SUM(...)`. The nostalgia has no separate
execution path.

Aggregates count or use numbers only, ignoring text, booleans and blanks.
Direct arithmetic treats blank as zero and booleans as zero or one; it
rejects text. `IF` evaluates only its selected branch. A circular formula,
division by zero, unsupported function or non-finite result is an explicit
error. Recalculation errors reject an edit batch atomically.

Reference copying respects `$A$1`, `$A1` and `A$1`. A copy moves a range to
an address; fill tiles a source range over an exact-multiple destination
range. Overlap uses the original source values. You do not need to practice
your slash-menu fingering first.

Sort operates on rows with explicit keys and optional header preservation.
The initial implementation rejects formulas inside the sorted data rows;
formulas outside it keep their addresses and recalculate against the new
values. This is a documented limitation, not a surprise trip to cell `ERR`.

## The context budget is also a budget

Reads accept a range, selected columns, one typed row filter, a row limit
and an offset. `next_offset` continues the physical rows of the same range,
including rows skipped by a filter. Omitted ranges use occupied bounds.
Use an explicit range when a sparse workbook spans distant cells.

The host sets workbook, text, formula, calculation and response limits.
Limits apply across a complete call. A rejected operation leaves the source
workbook unchanged. A response that would exceed its byte limit is refused;
request fewer rows or narrower columns. Even a spreadsheet from the floppy
era should know when its disk is full.

## File, save, survive

`Workbook::to_json` and `Workbook::from_json` validate the versioned payload.
They do not choose a database or a file path. MobKit owns persistence and
access control; the native adapter uses its configured storage provider.
An acknowledged durable write must survive a host restart. Please keep the
imaginary disk in the imaginary drive until the real transaction commits.

The full access and concurrency contract is in [SPEC.md](SPEC.md). A fork
shares the original workbook through current permissions. It inherits data
access, not the corner office. Owners can grant ordinary children access,
and revocation applies to subsequent calls.

WK1, Lotus macros, printer drivers and a mouse tutorial are outside this
release. The only paper jam should be in your historical imagination.
