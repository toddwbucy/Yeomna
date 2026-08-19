# Specification: 013 Graph and Database Lifecycle Verbs

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 3, under V-Q4 (immutable
in structure, not in content). The third of H2's seven phases.
Status: draft, 2026-08-17. **Both open rulings agreed by Todd 2026-08-17
as recommended, refined below by one measurement.**

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust and SQL keep
their syntax.

---

## Overview

Ten verbs. Seven graph verbs over the registry and the edge tables:
`graph.traverse`, `graph.neighbors`, `graph.shortest-path`, `graph.list`,
`graph.create`, `graph.drop`, and `graph.materialize`. Three database
lifecycle verbs under V-Q4: `database.list`, `database.create`, and
`database.drop`.

Two firsts ride along. `graph.drop` is the first destructive verb, and
the traversal verbs are the first instrument that can measure M2, whose
corpus has been waiting since the H3 dogfood.

## The rulings, as agreed 2026-08-17

**R13. Database lifecycle runs as `yeomna_provision`.** A fourth role:
`CREATEDB`, `NOLOGIN`, no grants on any KG table. `yeomna_app` is granted
membership and the verb layer wraps lifecycle statements in `SET ROLE
yeomna_provision` and `RESET ROLE`, so the runtime role never holds
CREATEDB itself and the owner role, which is superuser, never touches the
verb layer. Considered and declined: granting CREATEDB to `yeomna_app`
(administrative power on the runtime role) and using an owner connection
(hands the verb layer superuser for two verbs).

**R14. `graph.materialize` keeps its name and refuses.** The wire name is
binding under R4, and nothing anywhere defines what materialization does,
since the name arrived from the reference without semantics. It returns
`Unimplemented` naming what it waits for: a consumer that defines it, per
the no-vocabulary-ahead-of-a-consumer rule. It joins `hybrid`,
`structural`, and the graph-embed verbs in the refusing-by-name column.

**R13a, the refinement a measurement forced.** The kg pattern cannot be
stamped by applying `schema.sql` in a fresh database: pgvector is not a
trusted extension (measured on this cluster, no `trusted` line in
`vector.control`), so `CREATE EXTENSION vector` in a new database needs
superuser, which the provision role must never be. The stamp is therefore
a **template database**: `yeomna_template`, created once at cluster
provisioning by the owner with the full schema applied and marked
`IS_TEMPLATE`, and `database.create` with kind `kg` becomes
`CREATE DATABASE x TEMPLATE yeomna_template`. Postgres copies extension,
tables, grants, and ownerships in one step, a non-superuser may copy any
database marked as a template, and a stamped database is structurally
identical to the primary by construction rather than by re-execution.
Kind `plain` uses the default template. The template is re-stamped at
provisioning time whenever `SCHEMA_VERSION` bumps, which is a documented
operator step like role creation, not runtime machinery.

## Files to Modify

- `crates/yeomna-verbs/src/graph.rs` (new): the seven graph verbs' SQL.
- `crates/yeomna-verbs/src/database.rs` (new): the three lifecycle verbs.
- `crates/yeomna-verbs/src/execute.rs`: dispatch gains Phase 3, and the
  session gains the SET ROLE wrapper.
- `crates/yeomna-verbs/src/lib.rs`: module wiring.
- `crates/yeomna-verbs/tests/graph_verbs.rs` (new),
  `tests/database_verbs.rs` (new): contract tests.
- `crates/yeomna-verbs/tests/audit.rs`: the completeness list gains the
  Phase 3 verbs, per its own instruction.
- `crates/yeomna-store/schema.sql`: the prerequisites comment gains
  `yeomna_provision` and the template.
- `CLAUDE.md`: the cluster table gains the role and the template.

## Files to Reference

- `crates/yeomna-verbs/src/read.rs`: the validated-closed-set pattern for
  `kind`, reused for `basis` and `relation`.
- `crates/yeomna-store/tests/schema_claims.rs`: claim 1, whose pruning
  guarantee the traversal must preserve, and claim 4, whose cascade is
  what `graph.drop` rides.
- `docs/PRD-postgres-store.md`: D7 (UNION dedup, the enumerating
  exception and its cap), Phase 3's traversal contract.
- `docs/PRD-verb-layer.md`: V-Q4, the destructive-verb force pattern.

## Patterns to Follow

- Every value bound, except closed-set literals validated first: `basis`
  values are interpolated as literals **after** validation against the
  enum's three values, because parameterized basis arrays reach the
  planner too late for the compile-time partition pruning claim 1 proves.
  `relation` is validated against the CHECK set and bound as a parameter,
  since no partition hangs on it.
- The audit write, session, and envelope exactly as Phase 2 built them.
- The cluster-gated skip pattern, extended: Phase 3 lifecycle tests also
  skip when `yeomna_provision` or `yeomna_template` is absent, naming
  which.

## The traversal contract (D7 applied)

`graph.traverse` emits the claim 1 shape: a recursive CTE over
`(node, depth)` deduplicated by `UNION`, depth bounded by the request
(default 20), rows bounded by wrapping the walk in a `LIMIT` subquery
before aggregation, which stops recursion when the cap is reached rather
than after. The response reports each reached node once at its minimum
depth, plus a `truncated` flag when the cap was hit, because a capped
walk is an honest partial answer and a silent one is not.

`graph.shortest-path` is the enumerating exception: paths as arrays,
cycle-guarded by `<> ALL(path)`, the same capped-subquery bound, and
level-order recursion means the first arrival at the target inside the
cap is a shortest path. No path is an answer, not an error: `found:
false` in a success envelope. A truncated search that found nothing says
so, since "no path within the cap" and "no path" are different facts.

`graph.neighbors` is one hop, direction `out`, `in`, or `both`, same
filters, no recursion.

## The destructive pattern (`graph.drop`)

`force: false` is `Denied` naming the flag, so the acknowledgement is in
the audit args by construction. The drop is one `DELETE FROM graphs`,
which is T4 made mechanical through claim 4's cascade. The response
reports what was swept: node, edge, chunk, and embedding counts measured
before the delete. Naming swept contents in the audit trail itself is
Phase 6's requirement for `retire` and `prune` (G3 lists those, not
this) and lands with the Phase 4 transaction pattern.

## The lifecycle verbs (V-Q4 applied)

- `database.list`: name, owner, size, and whether it is a template, from
  `pg_database`. No role change needed.
- `database.create`: `SET ROLE yeomna_provision`, then
  `CREATE DATABASE x TEMPLATE yeomna_template` for kind `kg` or plain
  `CREATE DATABASE x` for kind `plain`, then `RESET ROLE` in all paths.
  The created database is owned by `yeomna_provision`, which is what
  makes `database.drop` work without superuser.
- `database.drop`: refuses by name, before any SQL, the session's own
  database, `postgres`, and any template (Denied). Otherwise `SET ROLE`
  and `DROP DATABASE`. A database the provision role does not own is
  `Denied` by Postgres itself, which protects the primary `yeomna`
  database from the verb layer permanently and by mechanism rather than
  by listing.

## Functional Requirements

1. Traversal preserves claim 1's pruning: with bases restricted to
   `declared` and `structural`, the emitted plan does not name
   `edges_asserted`, proven by an EXPLAIN in the test.
2. Traversal on the dogfood graph's cyclic `calls` edges terminates and
   returns each node once (D7's dedup doing its job on real cycles).
3. `graph.shortest-path` returns a shortest path when one exists within
   the cap, `found: false` when none does, and flags truncation.
4. An unknown basis or relation is `InvalidArgs` naming the closed set,
   never an empty result.
5. `graph.drop` without `force` is `Denied` and deletes nothing.
   With `force` it deletes everything claim 4's cascade covers and
   reports the counts.
6. `database.create` kind `kg` yields a database whose tables, grants,
   and extension match the primary (verified structurally by the test).
7. Every Phase 3 verb is in the audit completeness list and leaves
   exactly one terminal row per call.
8. `graph.materialize` returns `Unimplemented` naming R14.

## Edge Cases

- **EC-1.** Traverse from a key that does not exist: `NotFound`, distinct
  from a start node with no outgoing edges, which is one node at depth 0.
- **EC-2.** `graph.create` of an existing name: `InvalidArgs` naming it.
  Creating is not idempotent at the verb layer, because a caller who
  creates twice is confused and should hear so.
- **EC-3.** `database.drop` of a database with live connections: the
  Postgres refusal surfaces as `Denied` with the server's reason.
- **EC-4.** `database.create` when the template is missing (a
  half-provisioned cluster): the server error surfaces as `Internal`
  naming the template, so the operator learns which provisioning step
  was skipped.
- **EC-5.** `RESET ROLE` runs even when the lifecycle statement fails,
  so no error path leaves the session escalated.

## Implementation Notes

### DO

- One module per verb family, owning its SQL, as `read.rs` does.
- Wrap SET ROLE in a helper that owns the reset, so escalation is
  scoped by construction rather than by discipline.
- Test lifecycle verbs with test-named databases and drop them in the
  test, dropping before creating so a failed run cannot poison the next.
- Extend the audit completeness list in the same commit that adds the
  verbs, per the comment that file carries.

### DON'T

- Do not give the session an owner or superuser connection.
- Do not implement materialization, schema verbs, or the `sql` verb.
- Do not resolve M2. The instrument lands here, the benchmark is its
  own reported run.
- Do not consult the closed reference.

## Success Criteria

1. Workspace gate green with no cluster, tests skip, naming what is
   absent.
2. Against the cluster: all ten verbs answer, traversal proven on both
   a deterministic scratch graph and the cyclic dogfood graph.
3. The pruning EXPLAIN assertion passes at the verb level.
4. A kg-stamped database passes a structural comparison against the
   primary.
5. Review notes record response shapes as built, the provisioning steps
   run on the dev cluster, and any divergence.

## QA Acceptance Criteria

1. `cargo test --workspace` three times, clippy, fmt, all clean.
2. Editorial sweep clean.
3. Issue plus draft PR per the standing workflow.
