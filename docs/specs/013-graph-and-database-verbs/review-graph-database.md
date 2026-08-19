# Review Notes: graph and database lifecycle verbs

Reviewer: Claude, with Todd. Date: 2026-08-17. H2 Phase 3, and the first
destructive verb.

## What exists now

Ten verbs. The traversal family (`graph.traverse`, `graph.neighbors`,
`graph.shortest-path`) over the D7 contract, the registry family
(`graph.list`, `graph.create`, `graph.drop`), `graph.materialize`
refusing under R14, and the V-Q4 lifecycle family (`database.list`,
`database.create`, `database.drop`) running as the new provision role.

The dev cluster gained its two provisioning steps, run and verified
during the build: the `yeomna_provision` role (CREATEDB, NOLOGIN,
granted to `yeomna_app`, no grant on any KG table) and the
`yeomna_template` database (the schema applied by the owner, marked
IS_TEMPLATE). Both are recorded in the schema's prerequisites comment
and the CLAUDE.md cluster table.

## The response shapes as built

| Verb | `data` |
|---|---|
| `graph.traverse` | `graph`, `start`, `nodes` as key, kind, and minimum depth, `truncated` |
| `graph.neighbors` | `graph`, `key`, `neighbors` as key, kind, relation, basis, direction |
| `graph.shortest-path` | `found`, `path` as keys or null, `length`, `truncated` |
| `graph.list` | `graphs` as name, created_at, node and edge counts |
| `graph.create` | `graph`, `created` |
| `graph.drop` | `graph`, `dropped`, `swept` counts of nodes, edges, chunks, embeddings |
| `database.list` | `databases` as name, owner, is_template, size where readable |
| `database.create` | `database`, `kind`, `created` |
| `database.drop` | `database`, `dropped` |

## The rulings, as executed

**R13 held, and the escalation is scoped by construction.** Lifecycle
statements run inside a session helper that issues `SET ROLE
yeomna_provision`, runs the one statement, and resets on every path, so
no error leaves the session escalated (EC-5, asserted by test). The
primary database is protected twice: by name before any SQL, and by
mechanism, since the provision role does not own it and Postgres
refuses the drop itself. The foreign-database test proves the second
layer with a database the owner created.

**R13a held.** A kg stamp is `CREATE DATABASE ... TEMPLATE
yeomna_template`, and the structural comparison passes: ten tables, the
vector extension, `audit_log` owned by `yeomna_audit`, and the
`edges_identity` index, all present in the stamp because Postgres
copied them rather than because code re-created them.

**R14 held.** `graph.materialize` refuses, naming the consumer it waits
for, and the refusal is audited like everything else.

## Findings from execution

1. **The pruning proof reached the verb level, through GENERIC_PLAN.**
   Claim 1 proved partition pruning on a literal query. The verb
   parameterizes everything except the validated basis literals, and
   `EXPLAIN (GENERIC_PLAN)` plans exactly that statement with its
   placeholders unbound, so the test EXPLAINs the SQL the verb exposes
   for the purpose rather than a copy that drifts. The extended
   protocol demands parameter values, so the EXPLAIN goes through
   `simple_query`. The asserted partition is absent from the plan.
2. **The traversal terminates on the real cycles.** On the dogfood
   graph's `calls` edges, from a live starting symbol, the walk
   returns each node exactly once and completes well inside the bound.
   D7's dedup doing its work on the corpus M2 wanted, though M2 itself
   stays open until its benchmark is run and reported.
3. **EC-3 fired on the tests themselves.** The lifecycle tests held
   their own inspection connections open while asking for the drop,
   and the in-use refusal they got was the verb behaving correctly.
   The tests now release the connection first and retry briefly, since
   the backend notices teardown asynchronously.
4. **Phase 2's own guard caught the phase landing.** The EC-1 test
   asserting later-phase verbs refuse still listed `graph.list`, so it
   failed deterministically the moment Phase 3 implemented it. The
   case list now names verbs that remain future, with a comment saying
   the list shrinks as phases land. This is that test doing its job.
5. **A stamped database is owned by the provision role**, which is what
   makes `database.drop` work without superuser and what protects the
   primary. Recorded because it is load-bearing and invisible: the
   ownership comes from `current_user` at CREATE DATABASE time under
   SET ROLE.

## What is deliberately not here

Naming swept contents inside the audit trail belongs to Phase 6's
`retire` and `prune` per G3, with the Phase 4 transaction pattern.
`graph.drop` reports its sweep in the response and is audited as an
attempt and outcome like every verb. The `sql` verb, kg-pattern
detection for its refusal, and the schema verbs stay in Phases 4 and
H7. M2 has its instrument now and still has no benchmark.

## Verification

- 7 graph-verb tests and 5 database-verb tests green against the
  cluster, including the dogfood-cycle traversal and the structural
  stamp comparison.
- The audit completeness list covers all 22 implemented verbs, chosen
  for Phase 3 so refusals and misses prove auditing without mutating
  the cluster.
- Workspace gate 309 tests, three consecutive runs, clippy clean, fmt
  clean, editorial sweep clean.
