# Specification: 010 The Verb Contract

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 1. H2's first construction.
Status: draft, 2026-08-13.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust and SQL keep
their syntax.

---

## Overview

The verb vocabulary as types. A new crate, `yeomna-verbs`, carries the
closed `Verb` enum, a typed request struct per verb, the JSON envelope the
CLI capture pinned, and the error taxonomy. No I/O, no SQL, no daemon.
This is the phase that fixes the shapes every later phase speaks, closes
R4 name by name, and lands V-Q1's audit `outcome` column.

## Task Scope

- The `yeomna-verbs` crate: enum, requests, envelope, errors, round-trip
  tests.
- The R4 close: the binding name table below, exercised as enum variant
  names and their wire strings.
- The G6 close: every one of the 56 captured CLI commands gets its
  disposition in the table below.
- The `audit_log.outcome` column and its column-scoped UPDATE grant, in
  `yeomna-store/schema.sql`, with claim 6 extended to prove both sides.
- The no-SQL-outside-the-line workspace lint, as a test.

## Out of Scope

- Any verb implementation, any SQL emission, any I/O (Phases 2 through 6).
- Response structs beyond the envelope. **Amendment to the PRD's Phase 1
  sentence, with reasoning:** a response shape belongs to the phase that
  implements the verb, because a response typed ahead of the SQL that
  fills it is speculation, the same pre-modeling G4 declines elsewhere.
  The envelope is the Phase 1 response contract, `data` is typed per verb
  by its implementing phase, and the one already-ruled response shape
  (orient, V-Q3) lands with Phase 2 where its producer lives.
- The daemon, peercred, framing (Phase 5).
- The CLI repointing (Phase 7). `yeomna-cli` stays a held draft.

## Files to Modify

- `crates/yeomna-verbs/` (new): `Cargo.toml`, `src/lib.rs`,
  `src/verb.rs`, `src/envelope.rs`, `src/error.rs`.
- Root `Cargo.toml`: workspace member, `chrono` to workspace deps if not
  present.
- `crates/yeomna-store/schema.sql`: the outcome column and grant.
- `crates/yeomna-store/tests/schema_claims.rs`: claim 6 extension.

## Files to Reference

- `docs/PRD-verb-layer.md` v0.4: the rulings this spec executes.
- `origin/lift/007-cli` `crates/yeomna-cli/src/defs.rs` and `output.rs`:
  the captured argument surface and envelope convention.
- `docs/PRD-postgres-store.md`: the 32-verb inventory and the six
  ingestion operations.

## The R4 close: the binding name table

Wire names are dotted, lowercase, hyphenated. Presentation flags from the
CLI capture (`format`, `verbose`) are CLI-side rendering and never enter a
request. The captured `rerank` flag, which the CLI already refuses as
unavailable, is dropped rather than carried.

### Verbs (40)

| Group | Wire name | Was | Request carries |
|---|---|---|---|
| Orientation | `orient` | Orient | graph (optional, default survey of all) |
| | `status` | Status | nothing |
| | `health` | DbHealth | nothing |
| | `check` | DbCheck | key |
| | `stats` | DbStats | graph (optional) |
| | `codebase.stats` | CodebaseStats | graph |
| Read | `query` | DbQuery | search text, limit, kind (optional), hybrid flag, structural flag |
| | `get` | DbGet | kind, key |
| | `list` | DbList | kind (optional), limit, offset, parent key (optional) |
| | `count` | DbCount | kind (optional) |
| | `recent` | DbRecent | limit |
| Write | `insert` | DbInsert | kind, key, payload |
| | `update` | DbUpdate | kind, key, payload |
| | `delete` | DbDelete | kind, key |
| | `purge` | DbPurge | key |
| Graph | `graph.traverse` | DbGraphTraverse | graph, start key, relations, bases, depth, limit |
| | `graph.neighbors` | DbGraphNeighbors | graph, key, direction, relations, bases, limit |
| | `graph.shortest-path` | DbGraphShortestPath | graph, from key, to key, relations, bases, cap |
| | `graph.list` | DbGraphList | nothing |
| | `graph.create` | DbGraphCreate | name |
| | `graph.drop` | DbGraphDrop | name, force flag |
| | `graph.materialize` | DbGraphMaterialize | graph |
| Schema | `schema.apply` | SchemaCmd::Apply plus DbSchemaInit absorbed | target database (optional) |
| | `schema.list` | DbSchemaList | nothing |
| | `schema.show` | DbSchemaShow, absorbing DbCollections and IndexStatus | nothing |
| | `schema.version` | DbSchemaVersion | nothing |
| Database | `database.list` | CLI `db databases` | nothing |
| | `database.create` | CLI `db create-database` | name, kind (kg or plain) |
| | `database.drop` | new under V-Q4 | name, force flag |
| | `sql` | new under V-Q4 | database, statement |
| Embedding | `embed.text` | EmbedText | text, task |
| | `graph-embed.embed` | GraphEmbedEmbed | graph, key |
| | `graph-embed.neighbors` | GraphEmbedNeighbors | graph, key, limit |
| | `graph-embed.update` | graph-embed update (unverbed op) | graph, scope |
| Ingestion | `ingest` | CLI `ingest` (unverbed op) | path, graph, overwrite flag |
| | `codebase.ingest` | codebase ingest (unverbed op), absorbing CLI `codebase update` | path, graph, overwrite flag |
| | `codebase.retire` | codebase retire (unverbed op) | graph, path prefix, force flag |
| | `codebase.prune` | codebase prune (unverbed op) | graph, force flag |
| | `codebase.drift` | codebase drift (unverbed op) | graph, path |
| | `codebase.validate` | codebase validate (unverbed op) | graph |

### Dispositions that are not verbs

| Captured command | Disposition |
|---|---|
| `db create`, `db drop-collection` | removed, structure belongs to `schema.apply` (V-Q4: immutable in structure) |
| `db create-index` | removed, same reason |
| `db truncate` | absorbed into `purge` and `graph.drop`, the audited destructive verbs |
| `db export` | absorbed into `list` and `query`, jsonl rendering is CLI-side |
| `db collections`, `db index-status` | absorbed into `schema.show` |
| `extract` | CLI-local utility against the extraction service, touches no store, no verb (H5) |
| `embed service *`, `embed gpu *` | CLI-local process management, touches no store, no verb (H4) |
| `tools status`, `tools install` | CLI-local analyzer management, touches no store, no verb (H8) |
| `daemon` | starts the daemon, is not a verb the daemon serves (Phase 5) |
| `db query --rerank` | dropped, the capture already refuses it as unavailable |

## Functional Requirements

1. `Verb` is one enum, serde-tagged `{"verb": "<wire name>", "args":
   {...}}`, exhaustive, wire names exactly the table's.
2. Every request struct round-trips through serde with shape pinned by
   test, unknown fields rejected (`deny_unknown_fields`), so a client
   cannot smuggle an unrecognized argument past the contract.
3. The envelope is the captured convention: `{success, command, data,
   timestamp}` on success, `{success: false, command, error, timestamp}`
   on failure, timestamps RFC 3339 UTC.
4. The error taxonomy is `NotFound`, `InvalidArgs`, `Unimplemented`,
   `Denied`, `Internal`, each with a stable wire string usable as the
   audit `outcome` failure name.
5. No client-supplied actor field exists anywhere in the contract (PRD
   V3). A test proves deserialization rejects one.
6. `audit_log` gains `outcome text` (nullable, no default: a NULL outcome
   is an attempt whose completion was never recorded, which is V-Q1's
   crash story told by the schema). `yeomna_app` gains column-scoped
   `UPDATE (outcome)` and nothing else: history stays immutable, the
   completion mark does not.
7. The no-SQL lint: a workspace test asserting no SQL statement text
   appears in any crate's `src/` outside `yeomna-store` and
   `yeomna-verbs`.

## Edge Cases

- **EC-1.** An unknown wire name deserializes to an error, not a panic,
  and the error names the unknown verb.
- **EC-2.** A request with a client-supplied `actor`, at top level or
  inside `args`, is rejected by `deny_unknown_fields`.
- **EC-3.** `sql` targeting the KG database is representable in the
  contract (the refusal is runtime policy in Phase 4, not a type-level
  hole that would leak schema knowledge into the contract crate).

## Implementation Notes

### DO

- Keep the crate dependency-light: serde, serde_json, thiserror, chrono.
- Document every verb variant with its group, phase, and PRD ruling.
- Write the round-trip tests as one table-driven test over every
  variant, so adding a verb without its test fails visibly.
- Extend claim 6 in the same commit as the schema change.

### DON'T

- No I/O, no SQL, no store dependency in `yeomna-verbs`.
- No response structs beyond the envelope (amendment above).
- No `Db` prefixes, no ArangoDB vocabulary, anywhere in the crate.
- Do not consult the closed reference.

## Success Criteria

1. Workspace gate green everywhere (the new crate's tests need no
   cluster, the store's claim 6 extension keeps its skip gate).
2. All 40 wire names match the table exactly, proven by test.
3. Claim 6 proves the app role can mark an outcome and still cannot
   touch history.
4. The no-SQL lint passes on the current workspace and fails when fed a
   planted violation (proven by the lint's own negative test).
5. Review notes record the R4 close as executed and any divergence.

## QA Acceptance Criteria

1. `cargo test --workspace`, `cargo clippy --all-targets`,
   `cargo fmt --check`, all clean from the root.
2. Editorial sweep of spec and review notes clean.
3. Issue plus draft PR per the standing workflow.
