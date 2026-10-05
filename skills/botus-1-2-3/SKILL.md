---
name: botus-1-2-3
description: Store, calculate, query, and share persistent agent spreadsheets with Botus 1-2-3 through its two tools. Includes complete read, apply, operation, formula, and access references for agents without shell or file access.
---

# Botus 1-2-3

Use the discovered `botus_read` and `botus_apply` tools, including any
transport prefix in their actual names. The host supplies durable documents
and authenticated caller identity. This skill alone does not activate tools.
If the tools or required host services are absent, report the configuration
gap; do not claim data was saved or invent a local-file fallback.

## Load the reference for the job

Read the relevant bundled reference before first using that part of the
interface. These files are the complete functional reference, not setup
commands. All links stay inside this skill so it can be copied independently.

- [Read](references/read.md): all five `botus_read` actions, every query and
  filter field, defaults, responses, pagination, and help topics.
- [Apply](references/apply.md): all five `botus_apply` actions, request and
  receipt shapes, atomicity, revision conflicts, and exact retry rules.
- [Operations](references/operations.md): all eight batch operations, cell
  values, addresses, sheets, named ranges, copy/fill, and sorting behavior.
- [Formulas](references/formulas.md): complete syntax, functions, aliases,
  coercion, reference shifting, lazy evaluation, and error behavior.
- [Access](references/access.md): complete owner, principal, audience, grant,
  and deny JSON; private defaults, forks, ordinary children, and revocation.
- [Limits and errors](references/limits-and-errors.md): every configured
  limit, fixed bound, tool error, and cell error.

The host's skill/resource reader can load these references without shell
access. Botus also provides in-tool help through
`botus_read {"action":"help","topic":"operations"}`; topics are
`overview`, `operations`, `formulas`, `access`, and `limits`.

## Operating rules

Create or locate a workbook, read only needed cells, and retain the returned
document ID and current revision. Batch related edits atomically. Formulas
must be explicit objects or `formula` operations; a string beginning with
`=` remains text. Use a small `select` with an edit when it avoids a follow-up
read.

After an uncertain transport result, retry the same request ID and unchanged
request. After a revision conflict, read, reconcile, and use a new request ID.
A replay returns its original receipt, not current selected cells.

Workbooks default to agent-private ownership with permitted fork access.
Ordinary children need explicit or descendant sharing. Obtain a child's exact
`caller_principal` from its own `identity` read when selecting a grant
recipient. That principal is not authentication. Never send invented actor,
session, realm, ancestry, or administrator claims in tool arguments.

Recalculate like it is 1989. Keep the receipt like the process might restart.
