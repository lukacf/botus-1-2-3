# The little green spreadsheet manual

The screen is optional. The cells are not. This guide describes the pure
engine; the native tool adapter adds authenticated, revisioned persistence.

## Entering data

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
with `=` stay ordinary strings.

Empty cells, empty strings, zero and false are different values. Writing
`null` clears a cell. There is no special Lotus date serial or automatic
conversion of a numeric-looking string. Keep dates as ISO text for now.

## Recalculation

Recalculation is automatic. F9 is not required. Arithmetic, comparisons,
references, named ranges, `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT` and lazy `IF`
are supported.
The optional `@SUM(...)` spelling uses the same implementation as `SUM(...)`.

Aggregates count or use numbers only, ignoring text, booleans and blanks.
Direct arithmetic treats blank as zero and booleans as zero or one; it
rejects text. `IF` evaluates only its selected branch. A circular formula,
division by zero, unsupported function or non-finite result is an explicit
error. Recalculation errors reject an edit batch atomically.

Reference copying respects `$A$1`, `$A1` and `A$1`. A copy moves a range to
an address; fill tiles a source range over an exact-multiple destination
range. Overlap uses the original source values.

Sort operates on rows with explicit keys and optional header preservation.
The initial implementation rejects formulas inside the sorted data rows;
formulas outside it keep their addresses and recalculate against the new
values.

## Reading ranges

Reads accept a range, selected columns, one typed row filter, a row limit
and an offset. `next_offset` continues the physical rows of the same range,
including rows skipped by a filter. Omitted ranges use occupied bounds.
Use an explicit range when a sparse workbook spans distant cells.

The host sets workbook, text, formula, calculation and response limits.
Limits apply across a complete call. A rejected operation leaves the source
workbook unchanged. A response that would exceed its byte limit is refused;
request fewer rows or narrower columns.

## Saving and restarting

`Workbook::to_json` and `Workbook::from_json` validate the versioned payload.
They do not choose a database or a file path. MobKit owns persistence and
access control; the native adapter uses its configured storage provider.
An acknowledged durable write must survive a host restart. The adapter
refuses activation if the host cannot supply durable storage.

After an uncertain write result, retry with the same request ID and unchanged
request. MobKit returns the original durable receipt if the write committed.
After a revision conflict, read the current state, reconcile the change and
submit it with a new request ID. See the [apply reference](skills/botus-1-2-3/references/apply.md)
for the complete request and receipt rules.

The full access and concurrency contract is in [SPEC.md](SPEC.md). A fork
shares the original workbook through current permissions. Forks can inherit
read/edit access; ownership and administration remain with the owner.
Owners can explicitly grant ordinary children access, and revocation applies
to subsequent calls.

WK1 and Lotus macros are not supported.
