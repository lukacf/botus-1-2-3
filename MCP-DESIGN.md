# MCP without a second filing cabinet

Status: implementation contract. Installation and verification instructions
belong in the MCP guide once the implementation passes its acceptance tests.

The package supplies a real MCP server with exactly `botus_read` and
`botus_apply`, an installable stdio entry point, package manifests, and a
self-contained companion skill. The initial merged engine and native bundle
did not satisfy this delivery contract.

## Ownership

The spreadsheet engine and all Botus-specific protocol code stay in this
repository. MobKit owns the durable document service, identity resolution,
lineage and current authorization. Meerkat owns the tool dispatch context and
the normal MCP routing and execution lifecycle.

The host binds the MCP service to the same `ToolBundleContext` used by the
native adapter. Both paths execute the same Botus dispatcher against the
same document namespace. The relay does not open a database, persist caller
identity or infer an agent from a display name.

Registration is explicit. Disabled hosts and profiles expose zero Botus
tools. Profiles use either the native bundle or the MCP endpoint, avoiding
duplicate registration. Tool definitions remain compact; the skill contains
the complete functional reference and tools provide bounded on-demand help.

## Transport and authentication

The host runs a loopback MCP endpoint. The installed `botus-1-2-3 mcp`
entry point speaks MCP over stdio and relays to that endpoint. Protected
host configuration supplies the endpoint and a random relay credential.
Missing host configuration refuses startup. There is no local-storage or
anonymous-caller fallback.

The relay credential authenticates the relay connection, not the executing
agent. Actual caller authority is established separately for each call.
A normal MCP manifest alone cannot supply that authority. Installation must
configure both the host backend and its trusted dispatch-context provider;
the package makes no claim that an arbitrary unmodified MCP client can mint
host authority from this manifest.

Meerkat's optional per-call context provider receives the actual runtime
dispatch context and the exact selected destination. Its absence preserves
ordinary MCP behavior. A selected destination without required caller context
refuses before sending the call. Matching only a server's display name is
insufficient.

Any forwarded admitted origin uses the shared Meerkat `CallOrigin` /
`WireCallOrigin` contract. Requester attribution and the current executing
session remain distinct. Session identity cannot manufacture an admitted
human requester, grant chain or controller authority.

## One call, one authority binding

The external Botus provider mints an opaque, random, single-use call token.
A bounded ephemeral registry binds it to the host generation, physical
connection, actual executing session, operation, arguments and expiration.
Only the token and the defined origin projection cross the MCP boundary;
arbitrary turn metadata and conversation content do not.

The endpoint atomically consumes the token and verifies its request binding.
It then invokes the existing dispatcher, whose MobKit resolver checks current
identity, ancestry, native policy and document access. A previously minted
token cannot freeze an earlier grant. Arguments never establish caller
identity, regardless of authority-like field names.

An owned lifetime guard removes an unused token on completion, error or
cancellation. Registry capacity and retained bytes are bounded. Restart
invalidates tokens while preserving workbooks in the host store.
Cancellation does not promise rollback after a durable commit: a retry gets
a fresh transport token but retains the exact original Botus request ID and
arguments so the document service can return the original receipt.

## Acceptance

- Install the binary and package assets; initialize a real stdio MCP client,
  list exactly two tools, execute calls and shut down cleanly.
- Exercise parent, child and fork calls through the same destination with
  interleaved and concurrent identities. Confirm explicit child grants and
  inherited fork access, including current revocation.
- Reject absent, expired, replayed or request-mismatched tokens, unbound
  destinations and caller claims in tool arguments before storage access.
- Prove default-off native behavior, destination isolation, redacted errors
  and guard cleanup on cancellation and connection replacement.
- Restart the real persistent host and recover workbooks and proven fork
  access, while rejecting all earlier transport tokens.
- Preserve MCP tool error signaling and native execution policy. A read-only
  grant must not enable `botus_apply`.
- Include `plugin.json`, `mcp.json`, `SKILL.md` and every referenced manual
  page in the distribution. Verify the manual against actual request and
  response types.

F9 may be nostalgic. Authority is current.
