# The two-tool terminal

Botus exposes `botus_read` and `botus_apply` through a real stdio MCP server.
The spreadsheet engine, MCP relay and agent manual live in this package.
Meta owns workbook persistence and current agent access. There is no
second Botus database.

This package is not published to a package registry. Integration uses the
published Meerkat `=0.9.3` and `meerkat-meta =0.9.3` crates with
`extension-state`. The host and Botus must use the same native and Meta
versions so their runtime types agree. `Cargo.toml` records the
selections; the coordinated `Cargo.lock` is required for locked installation.
The consuming application does not need a native Git patch table.

Botus targets 0.9.x hosts. Earlier development builds used 0.8 storage and
continuity schema domains; opening those stores with 0.9 is not a supported
migration. Start this version with a fresh 0.9 host state directory, and keep
any existing development stores intact.

## Install the package

Build and install the executable from this repository:

```sh
./scripts/repo-cargo install --locked --path . --features mcp
python3 scripts/install-plugin-assets /your/plugin-directory/botus-1-2-3
```

No changes to `AUTOEXEC.BAT` are required.

The installer copies the manifests and the complete skill into a new
directory. It refuses to overwrite one. Installation does not activate a
workbook service or grant an agent access. The host must perform the
composition below and make the binary available to its MCP launcher.

## Compose the host

The [host example](examples/mcp_host.rs) registers the complete binding:

```rust
let host = Arc::new(BotusMcpHost::default());
let listener = host.start().await?;
let binding = listener.document_binding(
    relay_binary,
    ChildMcpAvailability::ChildAvailable,
)?;
let public_descriptor = binding.public_config().clone();
let builder = builder.register_mcp_tool_bundle_factory(binding);
```

Keep `listener` alive until the runtime has shut down. Select
`public_descriptor` in each authored agent profile that should have the two
tools. The builder installs the context provider into its agent factories;
it does not mount a duplicate native Botus bundle.

Document-bound hosts wait for configured MCP connection attempts before native
inherited tool visibility is resolved. The wait covers every MCP server
configured for that agent, using each server's `connect_timeout_secs`. A failed
connection completes the wait without making its tools available. This can
increase startup latency; hosts without document bindings keep asynchronous
MCP startup.

The public descriptor contains the executable and arguments, a nonsecret
binding marker, and empty environment slots. Only this descriptor belongs in
profiles. Meta fills the slots with the current host's endpoint and relay
credential after the session build hook, including child, fork and restored
session builds. A missing registration or changed descriptor refuses the
build. An old saved credential is never the restoration mechanism.

`relay_config()` is a lower-level host-only API that contains live
credentials. Do not persist its result, put it in a profile or log it.
Initial document bindings require canonical tool names; `tool_names` aliases
are refused so runtime classification and retained policy use the same names.

`ChildAvailable` supplies the tools to supported child profiles. It does not
share every workbook. `HostOnly` prevents access from agent-created child
mobs and implicit delegate mobs; copying a descriptor cannot bypass it.
Forks within the host mob remain eligible, subject to current workbook access.

## Retain both data and lineage

Configure the host's persistent state and extension document store as usual.
Activating an MCP document binding enrolls the local continuity store in
schema v3 identity history. Older v1/v2-only binaries then refuse that store;
review the [compatibility boundary](README.md#compatibility)
before activation. For the builder-owned definition path, retain the complete
mob store too:

```rust
let (storage, provenance) = persistent_mob_storage(mob_journal_path)?;
let builder = builder
    .persistent_state(state_directory)
    .mob_storage(storage, provenance);
```

Import `persistent_mob_storage` from
`meerkat_meta::mob_composition_manifest`. The paired store retains the mob
journal, identity state and fork records. `persistent_state()` alone does
not make mob lineage durable. On restart, reopen the same host stores and
mob journal, start a new listener, register its new binding, and reuse the
unchanged public profile descriptor.

A builder supplied with an already-created `mob_spec()` owns its session
factory and mob store. It refuses these late provider/storage overrides
instead of silently ignoring them. Use the builder-owned definition path
for this integration.

## Caller authority

Each actual native dispatch supplies its current executing session. A
single-use, bounded, short-lived correlation binds that session to the exact
connection, operation, arguments and native origin projection. The host
consumes it before consulting the existing current Meta resolver.
Workbook ownership and grants stay in the document service.

The current native origin codec emits `{"kind":"unavailable"}`. That value
makes no requester or represented-subject claim. Independently authenticated
executing-agent access still works; the wire value and correlation never
create additional permissions. Caller-like JSON arguments are not identity.

A plain third-party MCP client cannot manufacture this runtime context by
installing `mcp.json`. The configured host needs Meerkat's context provider.
The standalone stdio relay refuses missing host configuration.

## Verification and recovery

`make test-mcp` exercises the real stdio/HTTP transport with explicit fixture
authority. `make test-mcp-host` additionally exercises the actual Meta
composition and its runtime identities. Fixture tests alone do not establish
parent, child or fork integration. Consult the pull request's exact-head
validation record for completed qualification.

`make test-installed-mcp` installs a debug relay into a fresh temporary root,
checks the installed manifests and full skill byte for byte, then runs the
complete host suite against that installed executable. CI uses this gate.
The temporary installation is removed when the command exits. This verifies
source installation and real MCP behavior; it does not publish a package or
qualify a release-profile binary.

After an uncertain write result, use the same Botus request ID and unchanged
arguments. The runtime issues a new transport correlation. Meta's durable
receipt resolves whether the original write committed. Cancellation cannot
undo a write that already committed.

## Read the manual without file access

The [agent skill](skills/botus-1-2-3/SKILL.md) indexes complete functional
chapters. Call the existing read tool with
`botus_read {"action":"help","topic":"read"}` for all read actions or
`botus_read {"action":"help","topic":"apply"}` for all mutations. The
other chapter topics are `operations`, `formulas`, `access`, and `limits`.
Use the actual discovered tool name if a host prefixes it.

Each chapter is embedded from the same Markdown file shipped with the skill.
The full text appears in `help`, except `operations` and `limits` use `notes`;
their existing operation examples and live `limits` object remain available.
Help uses the normal authenticated MCP call path. It needs neither shell
access nor a separate skill-resource tool. The response bound applies to the
complete chapter and envelope; an oversized result returns `limit`, without
truncation or pagination. A missing host binding cannot be repaired by loading
the skill alone.
