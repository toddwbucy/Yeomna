# Specification: 012 Read Verbs Over the Live Schema

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 2. The second of H2's
seven phases.
Status: draft, 2026-08-15. **Carries three amendments to the parent PRD
and one ruling, all listed before the scope.**

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust and SQL keep
their syntax.

---

## Overview

Eleven verbs that read: `orient`, `status`, `health`, `check`, `stats`,
`codebase.stats`, `get`, `list`, `count`, `recent`, and `query`. Each
owns the SQL it emits, and `yeomna-verbs` becomes the second crate
allowed to hold SQL, which the workspace lint already anticipates.

This is the first phase where a verb executes, so three things arrive
with it that are not reads: the dispatch entry point, the audit write,
and the session context that carries actor and graph. Everything after
this phase inherits them.

There is a corpus to build against for the first time. `yeomna_self`
holds 1620 nodes and 2650 edges from the H3 dogfood, so these verbs are
developed and tested against a real graph rather than fixtures alone.

## Amendments to the parent PRD

**A1. `query` is search, not a filter engine.** The PRD's Phase 2 prose
describes "a structured filter (field, operator, value, limit, kind)
validated against a field allowlist." The contract spec 010 froze, and
R4 made binding, is `{search_text, limit, kind, hybrid, structural}`,
which came from the captured CLI and is what the command has always
been. The contract wins, and it is also the better shape: a
caller-supplied field and operator is a query language growing inside
the verb layer, which charter section 6 forbids. With no caller-supplied
field there is no allowlist to escape, and the only value reaching SQL
structure is `kind`, checked against the schema's own CHECK set.

**A2. The audit write lands here, not in Phase 4.** G1 and V2 require
every verb call to be audited, reads included, and this is the first
phase with calls to audit. What genuinely belongs to Phase 4 is the
transactional coupling of an audit row with a mutation and its diff-log
entry. The row itself, and V-Q1's outcome mark, are built here.

**A3. `embed.text` is not in this phase.** The PRD groups it under
Embedding rather than Phase 2, and it needs H4's embedder, which does
not exist. Named so its absence is not read as an oversight.

## Ruling: where the schema version comes from

`schema.version` and `orient` both report a schema version and nothing
in the store records one. This was the last gap holding Phase 2.

**Ruled: a compiled-in constant, `yeomna_store::SCHEMA_VERSION`, and no
stored marker.** A stored marker exists to detect drift between a
database and the binary, and drift is not a state this appliance
allows: spec 011 removed the in-place ALTERs because the graph is a
rebuildable index and a schema change costs a drop and a re-ingest. A
marker would invite the comparison that invites the migration. The
constant answers what the running binary's schema is, which is the
question `schema.version` is actually asked.

## Files to Modify

- `crates/yeomna-verbs/Cargo.toml`: `yeomna-store` and `tokio-postgres`
  become dependencies. Types-only ends here, by the PRD's design.
- `crates/yeomna-verbs/src/execute.rs` (new): the session context and
  the exhaustive dispatch.
- `crates/yeomna-verbs/src/audit.rs` (new): the audit row and its
  outcome mark.
- `crates/yeomna-verbs/src/read/` (new): one module per verb group,
  each owning its SQL.
- `crates/yeomna-verbs/src/lib.rs`: module wiring and exports.
- `crates/yeomna-store/src/lib.rs`: `SCHEMA_VERSION`.
- `crates/yeomna-verbs/tests/read_verbs.rs` (new): the contract tests.
- `crates/yeomna-verbs/tests/audit.rs` (new): G1 as a permanent test.

## Files to Reference

- `crates/yeomna-verbs/src/verb.rs`: the binding request shapes.
- `crates/yeomna-verbs/src/envelope.rs` and `error.rs`: the response
  contract and the five kinds.
- `crates/yeomna-store/schema.sql`: the tables, the grants, the
  `audit_log.outcome` column and its column-scoped UPDATE.
- `crates/yeomna-store/tests/schema_claims.rs`: claim 1 for the pruning
  a traversal depends on, claim 7 for `ts_rank_cd` answering over
  chunks.
- `docs/PRD-verb-layer.md`: V-Q1 attempt logging, V-Q3 orient's
  contract, V2 reads are audited, V3 the actor is not a request field.

## Patterns to Follow

- The cluster-gated test pattern: skip without a socket, skip without
  the runtime role provisioned, never fail the workspace gate.
- Parameter binding for every value. The one place SQL structure varies
  by input is `kind`, and it is validated rather than interpolated.
- Typed errors from the spec 010 taxonomy, never a stringly failure.
- `PgSink`'s honesty about connections: one session, one connection, and
  the M3 questions stay named rather than answered.

## The session context

```rust
pub struct Session {
    client: Client,
    actor: String,
    graph: Option<String>,
}
```

The actor is supplied at construction and is not reachable over the
wire, per V3. Phase 5 fills it from `SO_PEERCRED` and this phase's
constructor is what it will call.

`graph` scopes the document-read verbs, which carry no graph field of
their own because the captured CLI had none. When set, reads are
confined to it. When absent, reads span the database, which is correct
for `count` and `list` and ambiguous for `get`, handled in EC-2.

## The verbs

| Verb | Returns |
|---|---|
| `orient` | V-Q3's survey: per graph, node counts by kind, edge counts by relation and basis, embedding coverage and model, last ingest time, schema version. All graphs when none is named |
| `status` | Appliance identity: schema version, graph count, database name, whether the store answers |
| `health` | Integrity checks: nodes without chunks, chunks without embeddings, and the counts behind each, so the number is inspectable rather than a verdict |
| `check` | Whether a key exists, and in which graphs |
| `stats` | Row counts per table, scoped to a graph when named |
| `codebase.stats` | A graph's code shape: files, symbols by kind, edges by relation, analyzers seen |
| `get` | One node by kind and key, with its payload |
| `list` | Nodes by kind, paged, optionally under a parent |
| `count` | A count, by kind when named |
| `recent` | Nodes by `ingested_at` descending, which R9 made possible |
| `query` | FTS over `chunks.tsv`, ranked by `ts_rank_cd`, returning chunks with their node and rank |

`hybrid` and `structural` on `query` return `Unimplemented` naming what
they wait for, H4's embedder and H9's structural embeddings. They are
typed errors rather than silent degradation, because a search that
quietly ignores a requested ranking mode is worse than one that refuses.

## The audit write

Per V-Q1, and this is the mechanism every later phase inherits:

```
INSERT INTO audit_log (actor, verb, args) VALUES (...) RETURNING id
<the verb executes>
UPDATE audit_log SET outcome = 'ok' | 'failed: <kind>' WHERE id = ...
```

The row commits before the verb runs, so an attempt survives a crash
with a NULL outcome, which is what the column's nullability means. The
outcome update rides the column-scoped grant spec 010 landed.

`args` is the request as serialized by the contract. It carries what the
caller asked for and no actor, since the contract has no such field.

## Functional Requirements

1. Every verb in the table returns a well-formed envelope, success or
   failure, and never panics.
2. Every call writes exactly one `audit_log` row carrying actor, verb,
   args, and a terminal outcome. This is G1, and it is a test that runs
   over every implemented verb rather than a claim.
3. A failing verb marks its row `failed: <kind>` using the taxonomy's
   kind string, not its detail.
4. `query` ranks with `ts_rank_cd` over the generated tsvector and
   returns nothing a plain `LIKE` would have to scan for.
5. `kind` is validated against the schema's CHECK set before reaching
   SQL, and an unknown kind is `InvalidArgs`, not an empty result.
6. `orient` reports every field V-Q3 named.
7. Dispatch is exhaustive over `Verb`, so a variant added later fails to
   compile until it is handled or explicitly refused.

## Edge Cases

- **EC-1.** A verb of a later phase, reached through dispatch: returns
  `Unimplemented` naming the phase, never a panic or a silent success.
- **EC-2.** `get` with no session graph, for a key present in more than
  one graph: `InvalidArgs` naming the ambiguity and the graphs. A
  `NotFound` would be false and picking one would be worse.
- **EC-3.** `orient` on a graph that does not exist: `NotFound` naming
  it, distinct from an existing graph that happens to be empty, which
  is a survey of zeros.
- **EC-4.** An empty store: every verb answers rather than erroring. An
  appliance with nothing ingested is a valid state and `status` is how
  an operator learns that.
- **EC-5.** The audit insert itself failing: the verb does not run. A
  call that cannot be recorded is a call the appliance declines to
  make, which is what "one audited entry point" costs when the log is
  unavailable.

## Implementation Notes

### DO

- One module per verb group, each holding its own SQL, so the reader
  finds a verb's query where the verb is.
- Test against `yeomna_self` where a real graph makes the test better,
  and against a scratch graph where determinism matters more.
- Keep the dispatch match exhaustive and unadorned.

### DON'T

- Do not add a raw query surface, a field or operator parameter, or any
  path by which a caller composes SQL structure.
- Do not touch `IngestSink` or the write path.
- Do not implement the Phase 4 transaction. Reads audit, they do not
  need the mutation coupling.
- Do not consult the closed reference.

## Success Criteria

1. Workspace gate green with no cluster, tests skip.
2. Against the cluster: all eleven verbs answer, over both a scratch
   graph and the live `yeomna_self`.
3. The audit-completeness test covers every implemented verb.
4. `cargo test` proves an unknown `kind` and a later-phase verb both
   fail as typed errors.
5. Review notes record the response shapes as built and any divergence.

## QA Acceptance Criteria

1. `cargo test --workspace`, clippy, fmt, all clean from the root.
2. Editorial sweep of spec and review notes clean.
3. Issue plus draft PR per the standing workflow.
