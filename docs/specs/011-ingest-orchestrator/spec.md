# Specification: 011 The Ingest Orchestrator

Parent PRD: `docs/PRD-postgres-store.md`, Phase 4 (basis derivation), Phase 7
(idempotent writes, the enrichment protocol, D1 endpoint resolution). Fills
holes-ledger H3.
Status: built and merged-pending, 2026-08-15. **All five rulings agreed by
Todd 2026-08-15 as recommended.** See `review-orchestrator.md` for what
execution changed.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust and SQL keep
their syntax.

---

## Overview

The codebase ingest path, wiring the lifted crates to the store: walk a
tree, analyze each file, skip what has not changed, chunk, embed, and write
nodes and edges. It replaces `codebase_ingest.rs`, which was excluded from
the port as store-coupled, and it is new construction against tested parts
rather than a port.

It ends in the milestone the build order has been pointing at since the
lift: **the first dogfood ingest of this repository into a real graph.**

The document flow already exists (`yeomna-pipeline`'s orchestrator, spec
005) and is not rebuilt. This spec adds its sibling.

## What is already in hand

| Need | Supplied by |
|---|---|
| Per-file analysis, symbols, metrics | `yeomna_code::analyze_with_fallback` |
| Change detection | `FileAnalysis::symbol_hash`, SHA-256 over sorted symbol names |
| Provenance, which the schema requires NOT NULL | `FileAnalysis::analyzer` and `analysis_tier` |
| Chunk boundaries for code | `FileAnalysis::top_level_defs` |
| Deterministic keys | `yeomna-keys`, golden values pinned |
| Resumable, fault-isolated batches | `yeomna-batch` (`BatchProcessor`, `BatchState`) |
| Node, chunk, embedding writes | `PgSink`, spec 009 |
| Edge identity and its upsert target | `edges_identity`, spec 009 claim 8 |

## The gap this spec must close

**The sink cannot write edges.** `PgSink::route` knows four containers
(documents, codebase_files, chunks, embeddings) and `IngestSink` carries two
document-shaped methods. Nothing can reach the `edges` table.

This was deferred deliberately and twice: store PRD D1 says the sink owns
text-to-id resolution and assigns the strategy to "the Phase 7 spec," and
spec 009 declined to build an edge API because the sink's caller wrote no
edges. H3 is the caller that does.

## Rulings, all agreed 2026-08-15

**RULED. R8. Ingest logs only changes.** The sink compares the incoming
payload against the head and appends to `node_log` only on a difference,
so the log means what the charter says it means and an unchanged
re-ingest costs the history nothing. Considered and declined: writing no
log at all, which leaves a changed symbol with no record, and logging
every write, which grows the log by the node count on every run even
when nothing moved.

**RULED. R9. The ingest timestamp.** `nodes` and `chunks` carry no time column, so
`recent` and `orient`'s last-activity field have no source (found while
scoping H2 Phase 2). Recommended: add `ingested_at timestamptz NOT NULL
DEFAULT now()` to `nodes`, updated on upsert. One column, and it is the
column two ruled verb contracts already assume.

**RULED. R10. Where the orchestrator lives.** Recommended: `yeomna-pipeline`,
beside the document orchestrator, adding `yeomna-code` as a dependency. They
share the sink, the chunker, the embedder, and the crate is named for the
flow. A new crate for a few hundred lines buys nothing, and `yeomna-code`
stays independently useful either way.

**RULED. R11. The edge write path.** Recommended: **a fifth container route on the
sink**, not a new trait method. Edges arrive as JSON documents carrying
`from`, `to`, `relation`, `basis`, `analyzer`, and metadata, and the sink
resolves endpoints to `bigint` on the way in. This honors D1 (the sink owns
resolution) and leaves `IngestSink` untouched, which spec 005 asked for in
writing. The alternative, growing the trait, breaks a rule stated in the
trait's own documentation.

**RULED. R12. `edge_basis` on the Rust side** (this is R7, carried from spec 009).
Recommended: keep text at the boundary and cast in SQL, as the claim tests
do. A `postgres-types` derive buys type safety at the cost of a second
definition of the enum that must stay in step with the DDL, and the sink
already speaks JSON at that seam.

## Files to Modify

- `crates/yeomna-pipeline/src/codebase.rs` (new): the orchestrator.
- `crates/yeomna-pipeline/src/probe.rs` (new): the reads the write
  boundary does not carry.
- `crates/yeomna-pipeline/src/lib.rs`, `Cargo.toml`: module wiring, the
  `yeomna-code` and `ignore` dependencies.
- `crates/yeomna-store/src/sink.rs`: the symbol and edge containers,
  endpoint resolution, log-on-change, `IngestProbe`.
- `crates/yeomna-store/schema.sql`: `nodes.ingested_at`.
- `crates/yeomna-store/tests/codebase_ingest.rs` (new): the integration
  tests and the dogfood operation.

## Files to Reference

- `crates/yeomna-pipeline/src/orchestrator.rs`: the document flow, whose
  five-call store sequence and stale-delete this one mirrors.
- `crates/yeomna-code/src/lib.rs`: `analyze_with_fallback`, `FileAnalysis`.
- `crates/yeomna-code/src/{rust_imports,python_calls,cpp_edges,tree_sitter_edges}.rs`
  and `src/lsp/`: one edge resolver per language.
- `crates/yeomna-keys/src/lib.rs`: the golden keys both passes derive.
- `docs/PRD-postgres-store.md`: Phase 4's basis table, Phase 7's
  idempotency and enrichment protocol, D1.

## Patterns to Follow

- The document orchestrator's shape: config in, summary of counts out,
  per-item failures counted rather than fatal.
- The cluster-gated test pattern from specs 008 and 009: skip without a
  socket, skip without the runtime role, never fail the workspace gate.
- Deterministic keys from `yeomna-keys` for every node and edge, which is
  what lets the structural and semantic passes converge on one row.
- Sink containers as the only write path, per R11, so `IngestSink` stays
  the two methods spec 005 designed.

## Task Scope

- The codebase orchestrator: walk, analyze, hash-skip, chunk, embed, write.
- Basis derivation at ingest, per the parent PRD's Phase 4 table.
- The edge container on `PgSink`, with endpoint resolution and the
  forward-reference strategy below.
- `chunks.symbol_ids`, deferred from spec 009, filled here.
- Whatever R8 and R9 rule, applied.
- Integration tests against the sealed cluster, plus the first real ingest
  of this repository, reported.

## Out of Scope

- **Verbs.** `codebase.ingest` is a Phase 6 contract in the verb layer. This
  spec builds the machine, not its handle.
- The document flow, which exists.
- Late chunking, which waits on H4's embedder.
- Graph-embed and structural embeddings (H9).
- Any change to the analyzers in `yeomna-code`.
- The M3 window. Concurrent ingest stays unaddressed and named.

## The flow

1. **Walk.** Respect `.gitignore`, skip binaries, take a language filter.
2. **Analyze** each file through `analyze_with_fallback`, recording
   `analyzer`, `analysis_tier`, and `fallback_reason` for provenance.
3. **Hash-skip.** Compare `symbol_hash` against the stored head. Unchanged
   files skip analysis-dependent work. This is what makes a re-ingest of
   this repository cheap enough to run often.
4. **Chunk** using `top_level_defs` as boundaries.
5. **Embed** the chunks through `EmbeddingClient`.
6. **Write, in two passes.** All file and symbol nodes first, then edges.
   Cross-file edges name symbols in files not yet seen, which D1 predicted,
   and a global node pass is what makes resolution total rather than
   best-effort. Edges are accumulated during analysis and flushed after the
   node pass, so an edge naming an unresolvable endpoint is a counted error
   rather than a silent drop.

Basis is derived per the Phase 4 table: `defines` and resolved `imports` are
`declared`, language-server and tree-sitter `calls` and `implements` are
`structural`, and nothing this orchestrator writes is `asserted`, since
inference is H9's business. `analyzer` is never absent, which the schema
enforces and Q1 required.

## Functional Requirements

1. A re-ingest with no source change writes no new rows and reports the
   skip count.
2. The two-pass enrichment protocol survives: a symbol written structurally
   and then semantically converges to one row through the golden key, per
   Phase 7.
3. Every edge carries `basis` and `analyzer`, and duplicate edges collapse
   through `edges_identity`.
4. `chunks.symbol_ids` names the symbols each chunk covers.
5. Batch failures isolate per file, and a run is resumable through
   `yeomna-batch`'s state.
6. An unresolvable edge endpoint is counted and reported, never silently
   dropped.

## Edge Cases

- **EC-1.** A file whose analysis fails entirely: counted, reported, and
  the run continues. A repository with one unparseable file still ingests.
- **EC-2.** An edge naming a symbol that no analyzer produced: counted as
  unresolved, per FR 6.
- **EC-3.** A file deleted since the last ingest: out of scope here, since
  that is `codebase.retire`'s job (verb layer Phase 6). Named so it is not
  mistaken for an oversight.
- **EC-4.** Embedder unavailable **when embedding was requested**: the run
  fails loudly before writing, since a graph with nodes and no embeddings
  is a half-ingest that looks complete. Embedding is opt-in
  (`CodebaseConfig::embed`, default false) because no embedder ships until
  H4, so a run that never asked for vectors is not a half-ingest and does
  not fail.

## Implementation Notes

### DO

- Keep the walk, the analysis, and the write in separate functions, so the
  first dogfood run can be staged and inspected.
- Reuse the skip-without-a-cluster test pattern.
- Report counts the way `BatchSummary` already does.

### DON'T

- Do not add methods to `IngestSink`.
- Do not consult the closed reference. `codebase_ingest.rs` is excluded and
  its behavior is reconstructed from these contracts.
- Do not write `asserted` edges.
- Do not fix M3.

## Success Criteria

1. Workspace gate green with no cluster, tests skip.
2. Against the cluster: this repository ingests end to end, and the run is
   reported with node, edge, chunk, and embedding counts by kind and basis.
3. A second immediate ingest writes nothing new and says so.
4. Review notes record what the first real graph looked like, which is the
   first evidence M2 has ever had.

## QA Acceptance Criteria

1. `cargo test --workspace`, clippy, fmt, all clean from the root.
2. Editorial sweep clean.
3. Issue plus draft PR per the standing workflow.
