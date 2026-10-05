# Adversarial pure-engine review

Date: 2026-10-05. Scope: the uncommitted pure engine, its tests and `SPEC.md`.
The native adapter and storage integration are outside this review. No engine
or test implementation files were changed.

## Findings

### P2: Enforce the cell text limit on calculated values

Location: `src/engine.rs:1317-1322`, with literal-only enforcement at
`src/engine.rs:1008-1009`.

`max_cell_text_bytes` rejects an over-limit `Cell::Text`, but the evaluator
checks only the shared total byte budget on a calculated `Value::Text`.
Consequently, a host setting a five-byte cell limit rejects literal `123456`
while accepting the formula `="123456"` and returning its six-byte value.
Both `apply` and `read` accept the formula result. This violates the specified
per-cell text bound and makes the host's limit depend on how an agent supplies
the same cell content.

Enforce the per-cell limit on calculated text before memoizing/returning a
cell result, in addition to the existing shared materialization budget. A
focused regression should compare equivalent literal and formula-derived
text, including a lazy IF whose selected branch returns over-limit text,
and verify that a rejected edit leaves the workbook unchanged.

Resolution: the evaluator now checks calculated text against the per-cell
byte limit before returning or memoizing it. `tests/review_regressions.rs`
covers literal/formula equivalence, selected and unselected IF branches,
stored formula reads and atomic rejection.

### P2: Reject duplicate keys in persisted sparse maps before data is lost

Location: `src/engine.rs:273-279`; the derived `BTreeMap` deserializers are
declared at `src/engine.rs:62-69`.

The stored-workbook decoder accepts repeated keys in the sheets, cells and
named-range maps. Serde replaces the earlier entry before `validate` can
inspect it. A bounded probe with two `A1` entries, containing 1 and 2,
successfully loads a workbook containing only 2. Repeated sheet keys can
discard an entire earlier sheet. The specification says malformed stored
data must be refused rather than silently repaired or discarded.

Use a duplicate-rejecting map deserializer at stored identity/key boundaries,
or an equivalent lossless validation pass before constructing `Workbook`.
Add focused tests for duplicate cell, sheet and named-range keys. Ordinary
round trips from `to_json` must remain unchanged.

Resolution: sheets, cells and named ranges now deserialize through a map
visitor that rejects repeated keys before constructing a usable workbook.
`tests/review_regressions.rs` covers all three map boundaries and ordinary
JSON round trips.

## Evidence and limits

Read the complete engine, formula parser/evaluator, grid and value types,
plus current tests and usage documentation. Both findings were confirmed
with a small temporary harness outside the repository using:

```text
RUST_LANE_ID=adversarial-review ./scripts/repo-cargo run --quiet --offline --manifest-path /tmp/botus-adversarial-review/Cargo.toml
```

The harness completed successfully and observed the behavior stated above.
No broad test or build suite was rerun. The prior green baseline was supplied
by the implementation owner, not independently claimed here.

An initial concern that copied flat formulas would exceed parser nesting
limits was checked and disproved for valid chains through 64 terms; it is
not a finding. The prior copy/fill accounting fix, AST depth limit and shared
materialized-text budget were present in the inspected source.

Reviewed atomic batch replacement, sparse reads and pagination, formula/value
distinctions, typed filters, arithmetic/error paths, lazy IF, named ranges,
relative/absolute copying, and the documented refusal to sort formula rows.
No additional concrete defect was established in those paths. This review is
not a proof of complete spreadsheet compatibility or of the planned native
persistence/access behavior.

## Subsequent native-adapter review

### P2: Recover the original edit receipt after a concurrent delete

Location: `src/native.rs:447`, with receipt recovery only on revision mismatch
at lines 448-468. Independently reviewed on 2026-10-05; no native build run.

A duplicate edit can find no receipt, then the original identical edit commits
and the owner deletes the document before the duplicate reads it. The read
returns NotFound and the adapter exits before checking the newly committed
receipt. The provider retains that receipt and still authorizes the same owner
against the tombstone ACL, so a later retry succeeds while this one reports
unavailable. Repeat authorized receipt lookup before propagating an Edit read
failure. Keep the original failure if there is no currently authorized receipt.

Add a barrier-controlled test for this exact ordering. Assert the original
receipt with `replayed: true`, no selected cells or changes, and no weakening
of the existing refusal after authority is revoked.

Resolution: the Edit load-failure path now repeats the provider's authorized
receipt lookup before returning the original load error. A deterministic
test in `tests/native.rs` pauses the first missing-receipt result until the
original edit and deletion finish, then checks receipt-only recovery. It
also removes the editor grant before deletion and verifies that recovery
remains unavailable. Native all-targets compilation passed against the
coordinated upstream source overlay. Native test execution and clean pinned
dependency CI remain pending; compilation does not establish runtime acceptance.
