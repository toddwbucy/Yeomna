# Specification: 019 The Ingestion Verbs

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 6, first half. Ruled by
R21 D1 (long-running verbs block), D2 (`drift` re-walks and
hash-compares, reports, writes nothing), and R17's precedent (a verb
that needs its own connection opens one). The sixth PR in R21's order.
Status: draft, 2026-09-09.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

H3 built the ingest orchestrators and spec 015 built the document one,
and neither is reachable except from an `#[ignore]` test. That is not a
tool, and it is the reason the WeaverTools stand-up cannot be done by
the product yet. This spec makes ingestion a verb.

Four land here: `codebase.ingest`, `ingest` (documents and the
`conforms` links), `codebase.drift`, and `codebase.validate`. The two
destructive ones, `codebase.retire` and `codebase.prune`, are the
second half and their own spec, because T3's truth rests on what they
name in their audit args and that deserves its own review.

Together these close the verification loop the graph was built for:
change code, re-ingest, ask the graph, and use `drift` to find out
whether the graph still matches the source before trusting an answer
from it.

## The connection, and why a second one

`Session` holds its client inside the lock that serializes calls (R16),
and the ingest orchestrators take a `PgSink`, which owns a `Client`.
Rather than reshape the session around one verb, an ingesting verb
opens its own connection to the same database for the duration of the
call and drops it at the end, exactly as the `sql` verb does under R17.
The session's audit row still brackets the whole operation, so a
crash-shaped ingest leaves the same NULL outcome any other verb would.

## Task Scope

- `codebase.ingest`: walk, analyze, hash-skip, chunk, write, through
  `ingest_codebase`. Reports the orchestrator's summary.
- `ingest`: the document flow, `ingest_documents` followed by
  `link_conforms`, reporting both summaries. The conforms pass runs
  here because a document ingest is where a corpus's claims arrive and
  the links are what make them reachable.
- `codebase.drift` (D2): re-walk and re-analyze the tree, compare each
  file's `symbol_hash` against what the graph holds, and report
  `changed`, `new`, and `missing` with the paths. **Writes nothing.**
- `codebase.validate`: the runtime checks the schema's constraints
  cannot express, chief among them **an edge whose endpoints live in a
  different graph than the edge itself**, which the foreign keys admit
  because they point at `nodes(id)` and say nothing about `graph_id`.
- `IngestProbe` grows one read, `stored_file_keys`, so drift can see
  what the graph holds without the pipeline learning SQL.
- A `drift` function in `yeomna-pipeline` sharing the orchestrator's
  walk, so the comparison is against what an ingest would actually
  write rather than against a second idea of it.

## Out of Scope

- `codebase.retire` and `codebase.prune` (Phase 6b, spec 020).
- `graph-embed.update`, which waits on H4 and H9.
- Embedding during ingest. The orchestrator's `embed` flag stays off,
  since no embedder ships.
- The language-server pass, which stays opt-in and off by default: it
  puts an external process in a verb's path and turns a two-second call
  into a minute-scale one. A later ruling can expose it as a request
  field when someone wants it.
- Document drift. `codebase.drift` is codebase-scoped by its name, and
  documents get theirs when a corpus asks for it.

## Files to Modify

- `crates/yeomna-verbs/src/ingest.rs` (new): the four verbs.
- `crates/yeomna-verbs/src/execute.rs`: dispatch, the Phase 6 refusal
  list shrinking to the two destructive verbs.
- `crates/yeomna-verbs/Cargo.toml`: `yeomna-pipeline`.
- `crates/yeomna-pipeline/src/codebase.rs`: the `drift` function.
- `crates/yeomna-pipeline/src/probe.rs` and
  `crates/yeomna-store/src/sink.rs`: `stored_file_keys`.
- `crates/yeomna-verbs/tests/ingestion_verbs.rs` (new).
- `crates/yeomna-verbs/tests/audit.rs`: four more entries in the G1
  completeness list.

## Files to Reference

- `crates/yeomna-pipeline/src/codebase.rs` and `documents.rs`: the
  operations being wrapped, and their summaries, which become the
  verbs' data.
- `crates/yeomna-verbs/src/sql.rs`: the per-call connection pattern.
- `crates/yeomna-store/schema.sql`: what the constraints already
  enforce, which is what `validate` must not duplicate.

## Functional Requirements

- **FR1** `codebase.ingest` ingests a tree into an existing graph and
  reports the summary. A graph that does not exist is `NotFound`:
  creating one silently would make a typo a new graph.
- **FR2** `ingest` runs the document flow and then the conforms pass,
  reporting both summaries under one envelope.
- **FR3** A path that is not a readable directory is `InvalidArgs`
  naming it, before any connection is opened.
- **FR4** `codebase.drift` reports `changed`, `new`, and `missing` with
  their paths and writes nothing, proven by a test that drifts a graph
  and then finds the graph unchanged.
- **FR5** `codebase.validate` reports the cross-graph edge count and
  the other checks below, with zero meaning clean, and it writes
  nothing.
- **FR6** D1 holds: the call blocks until the operation finishes. No
  job id, no polling, no verb vocabulary for either.
- **FR7** All four are audited like any verb, one row each, the outcome
  terminal.

## What `validate` checks

The schema already enforces basis and analyzer being present, edge
identity, node kinds, relation shape per partition, and referential
integrity. What it cannot express, and what this verb is for:

1. **Cross-graph edges.** `edges.graph_id` and the graphs of
   `src_id` and `dst_id` can disagree, because the foreign keys point
   at `nodes(id)`. A traversal would then walk out of its own graph.
2. **Chunks whose `symbol_ids` name nodes in another graph**, the same
   hole one level down.
3. **Embeddings whose model disagrees within a graph**, which makes a
   vector search compare incomparable things once H4 lands.
4. **File nodes without a `path` or a `symbol_hash`**, which drift and
   retire both need and which a hand-written insert could omit.

Each is a count plus a bounded sample of offending keys, so an operator
sees what to look at rather than only that something is wrong.

## Edge Cases

- **EC-1** An empty directory: a summary of zeros, success, not an
  error. Nothing to ingest is a fact, not a failure.
- **EC-2** A path that exists and is a file: `InvalidArgs`.
- **EC-3** A second ingest of an unchanged tree: everything hash-skips,
  the summary says so, and the diff log stays quiet (R8).
- **EC-4** `drift` against a graph that was never ingested: every file
  is `new`, nothing is `missing`, and that is the honest answer.
- **EC-5** `drift` after a file is deleted from the tree: that file is
  `missing`, which is exactly what `retire` will act on in Phase 6b.
- **EC-6** `validate` on an empty graph: all zeros, success.
- **EC-7** A session with no endpoint configured: `Internal` naming the
  gap, the same refusal `sql` gives, because an ingesting verb needs
  its own connection and cannot invent one.

## Implementation Notes

DO:

- DO open the ingest connection as `yeomna_app`, the runtime role, so
  an ingest has exactly the grants every other write has.
- DO return the orchestrator's own summary fields rather than a
  reshaped subset, since those names are already the ones the review
  notes and the reports use.
- DO bound `validate`'s samples, and say what the bound was.
- DO run the gate three times before calling it done.

DON'T:

- DON'T create a graph as a side effect of ingesting into it.
- DON'T add a job model, a progress stream, or a timeout (D1).
- DON'T let `drift` or `validate` write anything, including a diff-log
  entry or an `ingested_at` bump.
- DON'T turn on the language-server pass or embedding from a verb.

## Success Criteria

1. A tree ingests through the verb layer, and the graph it produces is
   the graph the operation test produced.
2. Drift reports the three categories correctly and leaves the graph
   byte-identical.
3. Validate finds a planted cross-graph edge and reports zero on a
   clean graph.
4. The Phase 6 refusal list names only `codebase.retire` and
   `codebase.prune`.
5. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR7 and EC-1 through EC-7, cluster-gated,
  per-cause skip messages.
- The no-SQL lint still passes, with `yeomna-verbs` on the allowlist
  and `yeomna-pipeline` off it.
