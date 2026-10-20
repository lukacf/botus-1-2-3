# Botus 1-2-3

![Botus 1-2-3: a beige computer, boxed software and floppy disks, with the slogan Recalculate like it is 1989.](assets/botus-1-2-3.png)

**Recalculate like it is 1989.**

A spreadsheet workspace for agents that have better things to do than ask
for shell access. The package name is `botus-1-2-3`; Rust imports use
`botus_1_2_3`.

Botus takes its cell addresses and ranges from Lotus 1-2-3, with Unicode
text and automatic recalculation.
See [SPEC.md](SPEC.md) for the implementation contract and acceptance tests.
The [little green spreadsheet manual](USAGE.md) covers values, formulas,
copying, sorting and bounded reads.

## Status and installation

Botus is available from source under **MIT OR Apache-2.0**, at your option.
The default Rust build provides
the standalone spreadsheet engine; the optional `mcp` feature adds the MCP
server and host integration.

Botus targets **Meerkat 0.9.x**. The host integration pins the published
Meerkat **0.9.3** and Meerkat Meta **0.9.3** crates in
[Cargo.toml](Cargo.toml). Use that same dependency set in the host.
The optional `meta` feature provides native tool registration; `mcp` includes
it and adds the stdio MCP relay. No 0.8 runtime or Git dependency is required.

Install the relay from crates.io:

```sh
cargo install botus-1-2-3 --version 0.1.0 --locked --features mcp
```

The [0.1.0 release](https://github.com/lukacf/botus-1-2-3/releases/tag/v0.1.0)
includes the portable plugin and complete skill bundle.
For installation and host setup, follow [MCP.md](MCP.md). Agents need a
configured host for persistent storage and authenticated access; the skill
and executable alone do not provide those services.

The companion [agent skill](skills/botus-1-2-3/SKILL.md) contains the complete
functional reference, with separate chapters for
[read](skills/botus-1-2-3/references/read.md),
[apply](skills/botus-1-2-3/references/apply.md),
[cell operations](skills/botus-1-2-3/references/operations.md),
[formulas](skills/botus-1-2-3/references/formulas.md),
[access control](skills/botus-1-2-3/references/access.md), and
[limits and errors](skills/botus-1-2-3/references/limits-and-errors.md).
Agents retrieve one complete chapter through the existing read tool, without
shell access: `botus_read {"action":"help","topic":"read"}` or
`botus_read {"action":"help","topic":"apply"}`. The other topics are
`operations`, `formulas`, `access`, and `limits`. All chapters travel with the
skill and are embedded in the host-side tool implementation from those same
files. Help still requires an active host binding and authenticated caller;
installing the skill alone does not provide tool access.

## Package assets

`plugin.json` and `mcp.json` use the Agent Plugins 1.0.0 format. The MCP entry
point is `botus-1-2-3 mcp`. The optional `mcp` Cargo feature builds that
executable; the default engine build remains independent of the runtime.
The [MCP host guide](MCP.md) explains composition, persistence and caller
authority. The [transport and caller-context contract](MCP-DESIGN.md)
specifies how each tool call is bound to its authenticated runtime context.

Copy the manifests and complete skill to a new plugin directory with:

```sh
python3 scripts/install-plugin-assets /your/plugin-directory/botus-1-2-3
```

This copies public package assets only. The executable must also be installed
on the host's PATH. The manifest does not supply a storage backend or caller
authority: the host must bind its Meta services and trusted per-call MCP
context. A skill describes how to use the tools; installing it does not
activate them. Host setup and wire-level acceptance remain separate from
asset installation.

## Three easy steps

1. Put structured data in a workbook.
2. Calculate, filter and share it with authorized agents.
3. Persist through Meta.

Two agent tools, `botus_read` and `botus_apply`, serve the whole workspace.
Bulk operations and bounded responses keep tool calls and context use small.

## A cell of one's own

Workbooks default to private ownership by the creating agent's host identity.
Forks inherit permitted data access to the same workbook; they do not receive
ownership.
Owners can explicitly share with children, other agents or a mob. Realm
ownership does not imply that every agent can read the document.

Meta supplies persistence and enforces access through its document store.
The engine itself remains independent of the agent runtime.

## Compatibility

The interaction model is inspired by Lotus 1-2-3 Release 2.2. Workbooks use
versioned JSON. WK1 and Lotus binary compatibility are not supported.

Native bundle or MCP document-binding activation opts the host's local
continuity store into schema v3 identity history. Earlier binaries which support only v1/v2 then refuse
that database. Ordinary opens and head-only migration do not opt in. See the
[native guide](NATIVE.md) for the compatibility boundary.

## Development

Use `make check`, `make test`, `make lint` and `make fmt`. Build caches stay
outside the repository through `scripts/repo-cargo`.

The sparse engine, formula evaluator, compact native adapter and tests are
implemented here. Start with the runnable [beige budget](examples/beige_budget.rs)
or the [native integration guide](NATIVE.md). The [engine review](REVIEW-engine.md)
records its findings and fixes; [the manual](USAGE.md) describes supported
operations and limits.

Host acceptance covers child cleanup, parent and fork identity, sharing,
read-only enforcement, revocation and separate-process restart. Run
`make test-installed-mcp` against the pinned runtime pair. Earlier 0.9.0
qualification failures remain separate from the 0.9.3 results.
WK1, macros and protected exports remain deferred.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and verification.

## License

Choose either the [MIT license](LICENSE-MIT) or
[Apache License 2.0](LICENSE-APACHE). Contributions are accepted under the
same dual license.
