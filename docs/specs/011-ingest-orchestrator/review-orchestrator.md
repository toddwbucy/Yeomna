# Review Notes: the ingest orchestrator

Reviewer: Claude, with Todd. Date: 2026-08-15. H3, and the first time
Yeomna has read itself.

## The dogfood run

This repository, ingested into the `yeomna_self` graph on the sealed
cluster:

| | Cold | Warm |
|---|---|---|
| Wall clock | 2.50s | 0.95s |
| Files seen | 63 | 63 |
| Files written | 63 | 0 |
| Files skipped | 0 | 63 |
| Files failed | 0 | 0 |
| Symbols | 955 | 0 |
| Edges | 955 | 0 |
| Chunks | 204 | 0 |

Nodes by kind: 676 callable, 124 type, 84 module, 71 value, 63 file.
Edges: 955, all `defines / declared`. Analyzers: syn 926, rustpython
29. Zero unresolved endpoints, zero failures.

The warm run is the ruling working: every file's `symbol_hash` matched,
nothing was rewritten, and the history stayed quiet.

## The finding that matters: the graph has no depth

**Every edge is `defines`. There is not one cross-file edge, so the
graph is 63 stars rather than a web, and M2 still has no corpus.**

The cause is upstream of this spec and worth stating precisely.
`tree_sitter_edges::resolve` is the cross-file resolver, and it reads
`symbol.metadata["calls"]`. Nothing populates that field for Rust or
Python: a grep across `yeomna-code` finds the field consumed in
`tree_sitter_edges.rs` and `cpp_edges.rs`, and produced only inside
`cpp_edges`'s own fixtures. The semantic analyzers that ran here (syn
for Rust, rustpython for Python) were selected because they are the
highest fidelity available, and neither emits call metadata.

So the resolver ran over all 63 files and correctly produced nothing.
The orchestrator is doing what it was told, and what it was told is
not enough for a graph with depth.

This was not visible at spec time. The spec assumed the existing edge
producers would supply cross-file structure, and that assumption did
not survive contact. It is recorded rather than fixed here because the
spec's own DON'T list forbids changing the analyzers, and because the
fix has a design question in it (below) that belongs to Todd.

**What it would take.** `rust_imports::collect_use_paths` already
extracts use paths from Rust `Import` symbols, and the symbol map the
orchestrator builds already indexes every symbol by name across the
corpus. Resolving the last segment of a use path against that map,
under the same unambiguous-match-only rule `tree_sitter_edges` uses,
would produce real `imports` edges for this repository without
touching an analyzer. Call edges are the larger piece and want call
metadata from the analyzers themselves, which is H11's 004 territory.

The open question is where that resolver belongs: in the orchestrator,
which keeps `yeomna-code` untouched, or in `yeomna-code` beside the
other edge producers, which is where a reader would look for it.

## Decisions as built

**R8, log on change.** The sink reads the head before writing and
appends to `node_log` only when Postgres reports the payload distinct.
The comparison is `IS DISTINCT FROM` in SQL rather than a Rust
equality check, because a jsonb round trip through serde reports
differences that are only formatting. Entries are
`{"op":"insert","to":{...}}` and `{"op":"update","from":{...},
"to":{...}}`, a shape the PRD left open and this spec had to pick: it
satisfies "what a node was on entering, every change since," and the
head is derivable by taking the last `to`, which a test asserts
directly.

**R9, `ingested_at`.** On `nodes`, updated on every upsert, which
means it records when content last landed rather than when a file was
last looked at. An unchanged re-ingest does not move it, so `recent`
will mean recently changed.

**R10, sited in `yeomna-pipeline`** beside the document orchestrator.
One wrinkle the ruling could not have seen: hash-skip is a read, and
`IngestSink` is write-only by design. Rather than grow that trait,
which its own documentation forbids, a second small trait
(`IngestProbe`, one method) carries the read. The store implements
both.

**R11, edges as a fifth container.** `codebase_edges` routes to the
`edges` table and the sink resolves endpoints to `bigint`, which is
D1's assignment. Collection-qualified prefixes are stripped, and only
recognized container names are stripped, so a natural key containing a
slash survives. `IngestSink` is unchanged.

**R12, `edge_basis` as text.** Cast at the SQL boundary. No second
definition of the enum exists to drift from the DDL.

## Findings from execution

1. **`ALTER TABLE ... ADD COLUMN IF NOT EXISTS` deadlocks a concurrent
   test suite.** It takes an ACCESS EXCLUSIVE lock *before* evaluating
   the IF NOT EXISTS, so every `apply_schema` fought readers, and three
   claims failed with SQLSTATE 40P01 the first time the new column
   landed. Both that ALTER and the `audit_log.outcome` one from spec
   010, which had the same latent hazard and had only been lucky, are
   now guarded by a catalog check inside a DO block. The lock is taken
   only when the column is actually missing.
2. **Skipped files still feed resolution.** A file whose hash matched
   contributes no writes but does contribute its symbols to the map,
   because an edge pointing into an unchanged file must still resolve.
   Getting this wrong would have made warm runs silently lose edges.
3. **Chunk-to-symbol linkage crosses a unit boundary.** Chunk offsets
   are bytes and symbol positions are lines, so the orchestrator
   converts rather than comparing across units.
4. **Embedding is opt-in, and the spec said the opposite.** EC-4 asked
   for a loud failure when the embedder is unreachable, which is right
   when embeddings were requested and wrong as a default, because no
   embedder ships until H4 and the graph is worth building without
   vectors. `CodebaseConfig::embed` defaults to false, and EC-4 applies
   when it is true. The dogfood run wrote zero embeddings by design.

## Verification

- 7 orchestrator integration tests green against the cluster, covering
  the tree ingest, provenance on every edge, chunk-symbol linkage, the
  warm-run skip with history untouched, a changed file logging one
  update whose `to` reproduces the head, EC-1's unparseable file, and
  edge payloads surviving as jsonb.
- Workspace gate 280 tests, clippy clean, fmt clean.
- The dogfood graph is live on the dev cluster as `yeomna_self`.
