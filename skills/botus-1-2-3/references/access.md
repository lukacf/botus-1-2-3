# Ownership and sharing reference

Retrieve this complete chapter without file access using
`botus_read {"action":"help","topic":"access"}`; the text is in `help`.

Access is enforced by the host's document service on each call. Botus never
accepts caller identity, realm, mob membership, ancestry, or administrator
status as tool arguments. IDs below select resources or recipients; none
are credentials. Replace illustrative IDs with exact host-issued values.

Creation without `owner` or `access` defaults to ownership by the current
caller, `owner_reach: "forks"`, and no explicit grants or denies. Other peers
and ordinary spawned children have no automatic access. A permitted fork
accesses the same document ID, not a copied workbook.

## Principals

A Principal identifies an agent or a worker. Its complete JSON forms are:

```json
{"kind":"agent","value":"stable-agent-identity"}
```

```json
{"kind":"worker","value":{"mob_id":"native-mob-id","member_id":"native-member-id","creation_id":"native-creation-id"}}
```

All three worker fields are required strings. A worker identity includes its
creation ID; reusing a member name does not recreate the same principal.
Unknown fields reject. Identifier strings are nonempty, at most 512 UTF-8
bytes, without control characters.

Obtain the intended child's exact object from its own
`botus_read {"action":"identity"}` response's `caller_principal`. Do not
construct a Worker from display names or session IDs. A parent receiving
that value may choose it as a recipient but cannot authenticate as the child.

## Owners

Ownership controls `share`, `transfer`, and `delete`, separately from data
read/edit grants. The complete Owner forms are:

```json
{"kind":"agent","value":{"kind":"agent","value":"stable-agent-identity"}}
```

```json
{"kind":"agent","value":{"kind":"worker","value":{"mob_id":"native-mob-id","member_id":"native-member-id","creation_id":"native-creation-id"}}}
```

```json
{"kind":"mob","value":"host-mob-id"}
```

```json
{"kind":"realm"}
```

An agent Owner's `value` is a complete Principal, not just its string or
display name. A mob Owner requires host-established mob management authority;
realm ownership requires host realm administration. A requested owner is
validated by the host. Creating for an owner also requires permission to
manage that owner. Transferring requires permission to manage the current
owner plus a valid destination; it can remove the transferor's access.

Mob ownership does not give every mob member data access, and realm ownership
does not make a workbook public. The owning agent or host-authorized manager
has owner-derived data rights, subject to current host policy and data denies.
Broader audiences require grants.

## Audiences

Every grant and deny has an `audience`, with one of these complete forms:

```json
{"kind":"agent","value":{"principal":{"kind":"agent","value":"recipient-identity"},"reach":"self_only"}}
```

```json
{"kind":"mob","value":"host-mob-id"}
```

```json
{"kind":"realm"}
```

The `agent` audience accepts either Principal form. Its `value.principal` is
required, and `value.reach` is optional with default `forks`. Null rejects.
A `mob` audience matches current membership of that exact mob, not arbitrary
members of child mobs. A `realm` audience matches callers within the host-bound
realm; it is not global cross-system access.

### Reach

| Value | Matching principals |
| --- | --- |
| `self_only` | Only the named principal |
| `forks` | Named principal and descendants reached entirely through native fork edges |
| `descendants` | Named principal and descendants through native fork or spawn edges |

Reach is used by agent audiences and by `owner_reach` for an agent-owned
workbook. The default `forks` is transitive through forks, but stops at a
spawn edge. All reach depends on host-proven native lineage, not a claimed
relationship in a message. Data rights are capped by the current effective
policies of the caller and delegation sources. They cannot amplify a read
grant into an edit grant, or a data grant into ownership administration.

## Access object

The complete shape is:

```json
{"owner_reach":"forks","grants":[],"denies":[]}
```

| Field | Type | Default |
| --- | --- | --- |
| `owner_reach` | Reach string | `forks` |
| `grants` | Array of Grant objects | `[]` |
| `denies` | Array of Deny objects | `[]` |

All fields are optional, none accepts `null`, and unknown fields reject.
The combined grant/deny count may not exceed 256. `{}` means the defaults,
not preservation of existing access. A share action replaces the complete
object. A transfer preserves it only when `access` is omitted or `null`.

A Grant requires `audience` and `role`, without extras:

```json
{"audience":{"kind":"agent","value":{"principal":{"kind":"agent","value":"recipient-identity"},"reach":"self_only"}},"role":"reader"}
```

`role` is exactly `reader` or `editor`. Reader grants permit reads. Editor
grants permit reads and edits, subject to host policy. Neither permits
sharing, transfer, or deletion. Matching grants combine using the highest
allowed data role, after lineage policy ceilings.

A Deny requires `audience` and `operation`, without extras:

```json
{"audience":{"kind":"agent","value":{"principal":{"kind":"agent","value":"revoked-identity"},"reach":"descendants"}},"operation":"read"}
```

`operation` is exactly `read` or `edit`. A matching read deny blocks both
read and edit; an edit deny blocks only edits. Data denies take precedence
over owner-derived data access and grants. Denies do not themselves transfer
ownership or remove an owner's administrative authority, which remains
separately subject to host policy. A denied owner may still repair the ACL
if the host permits management.

## Sharing with one child

1. Obtain that child's current `caller_principal` from its own identity read.
2. Describe the workbook to obtain its current revision and complete access.
3. Preserve intended existing grants and denies, then add a Grant whose
   audience contains the exact child principal. Use `reach: "self_only"`
   unless further delegation is intended, and choose `reader` or `editor`.
4. Apply `share` with a fresh request ID and the current revision.

For example, if the exact reported principal is the Worker below, this is
the full share request for a workbook with no other intended grants/denies:

```json
{"action":"share","request_id":"share-child-unique-1","id":"document-id","revision":"current-revision","access":{"owner_reach":"forks","grants":[{"audience":{"kind":"agent","value":{"principal":{"kind":"worker","value":{"mob_id":"reported-mob","member_id":"reported-member","creation_id":"reported-creation"}},"reach":"self_only"}},"role":"editor"}],"denies":[]}}
```

The example's strings must be replaced with reported values, not guesses.
An ordinary child's presence or knowledge of a workbook ID gives no access.

## Sharing with descendants and revocation

To make owner-derived data access available to all permitted ordinary
children and forks, use `owner_reach: "descendants"`, preserving other
intended ACL fields. To remove that inherited route, replace `owner_reach`
with `self_only`. Explicit grants can still authorize those same callers,
so remove or narrow them as needed. A matching deny can block a caller or
subtree across other data routes.

Revocation affects subsequent operations and receipt lookups. It does not
erase values an agent already received. Current ACL and revision are checked
at the mutation boundary; a cached description is not current authority.

Deleting or resetting an agent does not silently delete its workbook. Stable
agent ownership survives correctly configured host restoration. Temporary
helpers and their forks still retire with their native lifetime; durable
workbooks do not make ephemeral mobs durable. Missing native identity or
lineage authority refuses access rather than granting fallback access.
