# Contributing

Botus keeps its agent interface small: `botus_read` and `botus_apply`.
Read [SPEC.md](SPEC.md) before changing behavior, and follow [AGENTS.md](AGENTS.md)
for repository conventions. Keep the skill reference and embedded help in
sync with any interface change.

## Development

Use the pinned Rust toolchain and the repository's Make targets:

```sh
make fmt
make check
make lint
make test
```

The default build tests the spreadsheet engine without runtime dependencies.
`make test-mobkit` covers the native adapter. `make test-installed-mcp`
installs a relay in a temporary directory, checks its plugin and skill assets,
and exercises the actual host integration. These integration targets require
the exact dependencies in `Cargo.toml` and `Cargo.lock` and substantially more
build space than the engine tests. Use `scripts/repo-cargo` for targeted Rust
commands so build caches remain outside the checkout.

Preserve private-by-default ownership, authenticated per-call authority,
bounded output, atomic writes and durable retry receipts. Changes to sharing,
lineage or persistence need regression coverage for revocation and restart.
Keep runtime-version compatibility claims tied to actual host acceptance.

Open a pull request describing the behavior changed and the checks completed.
Do not include credentials, private workbooks or real user data in fixtures.

## License

Unless explicitly stated otherwise, contributions are licensed under
MIT OR Apache-2.0, at the recipient's option, as in [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE).
