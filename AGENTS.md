# Botus 1-2-3

SPEC.md owns the product contract. Changes must preserve its ownership,
lineage, persistence, context and bounded-execution guarantees.

Use ASCII hyphens only in authored text. Keep jokes in documentation,
examples and opt-in presentation. Errors and protocol data remain precise.

Use Make or ./scripts/repo-cargo for Rust work. Coordinate Cargo.lock and
builds; never run simultaneous Cargo commands against the same target.
Use package-scoped verification. Do not install an independent Botus
database in the MobKit integration or use session metadata as a workbook.

No caller identity, ancestry, realm or management authority may come from
model-controlled arguments. Read-only operations must not persist caches.
Private means private to the owner and permitted fork lineage, not peers.
Revocation, concurrent changes and process restart are first-class tests.

This is a functional homage, not a claim of Lotus binary compatibility.
Do not silently approximate unsupported formulas or discard data on export.

