# Formula language reference

Retrieve this complete chapter without file access using
`botus_read {"action":"help","topic":"formulas"}`; the text is in `help`.

Enter formulas with the `formula` operation or a cell object
`{"formula":"=SUM(A1:A9)"}`. A plain string such as `"=SUM(A1:A9)"` is text.
Within the formula string, surrounding whitespace is trimmed and one leading
`=` is optional. JSON escaping still applies outside the formula language.

## Literals and references

| Form | Meaning |
| --- | --- |
| `12`, `1.5`, `.5`, `1e3`, `1E-3` | Finite decimal/scientific number |
| `"text"` | Text literal; double a quote to include it: `"say ""hello"""` |
| `TRUE`, `FALSE` | Case-insensitive boolean literals |
| `A1`, `aa12` | Cell on the formula's current sheet |
| `$A$1`, `$A1`, `A$1` | Absolute column/row anchors for copying |
| `A1:B9`, `A1..B9` | Inclusive rectangular range; `..` is an alias for `:` |
| `Sheet1!A1`, `'Other sheet'!A1` | Reference to an explicit sheet |
| `'Owner''s sheet'!A1` | Escape an apostrophe in a quoted sheet name by doubling it |
| `DISK_COSTS` | Workbook-wide named range, case-insensitive |

Numeric literals and calculations use IEEE 754 binary `f64`, not exact
decimal arithmetic. Finite integers beyond `2^53` may round, as can decimal
fractions. Keep exact IDs as text; currency calculations have no special
decimal precision guarantee.

References must fit the host grid. Missing cells within an existing sheet
are blank; a nonexistent explicit sheet rejects workbook validation. Ranges
must run top left to bottom right, and cannot span different sheets. A first
endpoint's explicit sheet is inherited by its unqualified second endpoint:
`'Other sheet'!A1:B9`. Both endpoints may explicitly name that same sheet.

Use a named range directly in an aggregate. Scalar use requires exactly
one cell; a literal range expression, including `A1:A1`, requires an
aggregate and cannot stand alone as a scalar. A named range's definition is
not shifted by copying a formula.

## Operators and precedence

From highest to lowest precedence:

| Operators | Associativity / behavior |
| --- | --- |
| Parentheses | Explicit grouping |
| `^` | Power; right-associative |
| Unary `+`, `-` | Unary numeric sign |
| `*`, `/` | Left-associative multiplication/division |
| `+`, `-` | Left-associative addition/subtraction |
| `=`, `<>`, `<`, `>`, `<=`, `>=` | Left-associative numeric comparisons; return booleans |

Thus `2^3^2` is `512`, and `-2^2` is `-4`. Parenthesize a comparison rather
than relying on chained-comparison behavior. `!=`, `==`, `%`, concatenation,
logical infix operators, arrays, and implicit multiplication are unsupported.

Scalar arithmetic and comparisons use numbers directly, blank as zero,
`FALSE` as zero, and `TRUE` as one. Text is never parsed as a number, even
when it looks numeric. Division by zero and nonfinite results are errors.
Comparisons are numeric, not text comparisons; use read filters for typed
text filtering.

## Complete function set

Function names are case-insensitive. Every supported function accepts an
optional `@` prefix: `@SUM`, `@AVERAGE`, `@MIN`, `@MAX`, `@COUNT`, and `@IF`.
Both spellings use the same parser and evaluation rules.
Arguments are comma-separated; semicolons are unsupported.

| Function | Arguments | Result / empty numeric input |
| --- | --- | --- |
| `SUM(...)` | Zero or more scalars, direct ranges, or direct named ranges | Sum of numeric values; `0` when none |
| `AVERAGE(...)` | Same | Numeric sum divided by numeric count; error when none |
| `MIN(...)` | Same | Minimum numeric value; `0` when none |
| `MAX(...)` | Same | Maximum numeric value; `0` when none |
| `COUNT(...)` | Same | Count of numeric values; `0` when none |
| `IF(condition, then_value, else_value?)` | Exactly two or three expressions | Selected branch; omitted false branch returns boolean `false` |

Aggregates ignore text, booleans, and blanks, including literal arguments.
They do not count a boolean as zero/one as scalar arithmetic does. Scalar
expressions can be arguments: `SUM(TRUE, TRUE+0)` returns `1`. Errors from
evaluating a referenced cell/range propagate; they are not ignored as text.
Ranges/names are flattened only when they are direct aggregate arguments;
`SUM(A1:A9+1)` is not array arithmetic. `SUM` and `AVERAGE` reject a nonfinite
intermediate sum even if later values would mathematically cancel it.

`IF` converts its condition using the scalar numeric rule: zero is false,
any other finite number is true, and text is an error. It evaluates only
the selected branch. For example `IF(FALSE,1/0,7)` returns `7`, and
`IF(FALSE,A1,7)` stored in `A1` does not create an evaluated cycle.

Laziness does not bypass parsing or structural validation. Both branches
must have supported syntax/functions, and explicit cell references must
be within the grid and name existing sheets, even in an unselected branch.
An unknown function in an unused branch still rejects the formula.

## Copying and filling

`copy` and `fill` shift each relative cell/range coordinate by the difference
between its original cell and destination cell. `$` freezes its corresponding
coordinate. Explicit sheet names stay the same; their relative coordinates
still shift. Unqualified references in a cross-sheet copy refer to the
destination sheet. A shifted reference outside the grid rejects the batch.

Copied formulas are rendered in equivalent canonical syntax: parentheses
may be added, `@` removed, ranges rendered with `:`, names uppercased, and
explicit sheets quoted. Do not use exact original formula text as the test
for copying correctness. Formula view exposes the stored result.

## Evaluation, errors, and bounds

Evaluation is deterministic: no time, random, network, shell, file, external
workbook, volatile, lookup, or macro functions. There is no Excel or Lotus
binary/formula compatibility claim beyond the listed language.

All formulas in an edited candidate are evaluated before commit. Any
calculation error rejects that mutation. Reads of previously stored formulas
can instead return an error object for an individual selected cell:

```json
{"error":{"code":"divide_by_zero","message":"Division by zero"}}
```

A calculation error in a filter column fails the read; a budget exhaustion
also fails the entire read. Formula view returns explicit formula objects
instead of calculated output, although filters still calculate their column.

Formula bytes, parser nodes/tokens, nesting, dependency depth, evaluated work,
and calculated text have independent limits. A page shares one evaluator and
memo across its selected cells and filter evaluations; a mutation's formula
validation shares one evaluator across its workbook. No caches are persisted
by reads. See [all limits and errors](limits-and-errors.md).

## Examples

These formulas can be submitted as explicit formula cells:

```json
[{"op":"formula","at":"C1","formula":"=SUM(A1:A9)"},{"op":"formula","at":"C2","formula":"=@AVERAGE(A1..A9)"},{"op":"formula","at":"C3","formula":"=IF(B1=0,0,A1/B1)"},{"op":"formula","at":"C4","formula":"=IF(TRUE,\"say \"\"hello\"\"\",\"no\")"}]
```

The first two require suitable numeric source data; this is a syntax
example, not a guaranteed-valid batch for an empty workbook. A two-argument
`IF(FALSE,99)` returns boolean false, not blank or numeric zero.
