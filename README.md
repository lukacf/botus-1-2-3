# Botus 1-2-3

**Recalculate like it is 1989. Save like the process might restart.**

A spreadsheet workspace for agents that have better things to do than ask
for shell access. The package name is `botus-1-2-3`; Rust imports use
`botus_1_2_3`.

Botus takes its cell addresses, ranges and affection for recalculation from
Lotus 1-2-3. Its Unicode support has moved on from the floppy-disk era.
See [SPEC.md](SPEC.md) for the implementation contract and acceptance tests.

## Three easy steps

1. Put structured data in a workbook.
2. Calculate, filter and share it with authorized agents.
3. Persist through MobKit. Please do not remove the virtual disk while the
   imaginary light is on.

Two agent tools, `botus_read` and `botus_apply`, serve the whole workspace.
Bulk operations and bounded responses keep the context bill out of the
spreadsheet's expense column.

## A cell of one's own

Workbooks default to private ownership by a stable agent. Forks inherit
permitted data access to the same workbook; they do not receive ownership.
Owners can explicitly share with children, other agents or a mob. Realm
ownership does not imply that every agent can read the document.

MobKit supplies persistence and enforces access. Botus does not hide an
extra SQLite file behind the filing cabinet. The engine itself remains
independent of the agent runtime.

## Compatibility department

The specification is inspired by Release 2.2, not chained to its file
format. WK1 is an optional future novelty. No binary compatibility is
claimed. No lotus flowers, spreadsheets or 5.25-inch floppies were harmed
in the design of this package.

## Development

Use `make check`, `make test`, `make lint` and `make fmt`. Build caches stay
outside the repository through `scripts/repo-cargo`.

This repository begins with an implementation specification. Feature and
validation claims must track the code and recorded test results.

