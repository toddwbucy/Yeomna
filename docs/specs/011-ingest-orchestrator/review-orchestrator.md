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
| Symbols | 956 | 0 |
| Edges | 1169 | 0 |
| Chunks | 205 | 0 |

Nodes by kind: 677 callable, 124 type, 84 module, 71 value, 63 file.
Edges: 956 `defines / declared` and 213 `imports / declared`.
Analyzers: syn 1140, rustpython 29. Zero unresolved endpoints, zero
failures.

The warm run is the ruling working: every file's `symbol_hash` matched,
nothing was rewritten, and the history stayed quiet.

## Corrected: the wrong resolver was wired, twice over

The first run produced 955 edges, every one of them `defines`, and the
first write-up of these notes blamed an upstream capability gap. Todd
asked why tree-sitter was being used on Rust and Python at all. It was
the right question and the diagnosis was wrong on both counts.

**On analysis, tree-sitter never ran.** `analyze_with_fallback` picked
syn for Rust and rustpython for Python, which the analyzer counts show.

**On edges, only tree-sitter ran, and that was this orchestrator's
choice.** `yeomna-code` ships a resolver per language, and the one that
was wired is the fallback for languages that have nothing better:
`tree_sitter_edges::resolve` reads a `calls` metadata field that syn
does not write, so it correctly produced nothing on a Rust corpus.
Sitting unused beside it were `rust_imports::resolve_rust_imports`,
which reads the use statements syn already extracted, and
`python_calls::resolve_python_calls`, which reads call sites the Python
AST analyzer does record.

The claim in the first draft of these notes, that nothing populates
`metadata["calls"]` for Rust or Python, was false for Python and came
from a grep piped through `head -8` that truncated before reaching
`python.rs`, where line 284 writes exactly that field. The conclusion
was drawn from a cut-off list.

**After wiring each language to its own resolver**, the same repository
produces **213 cross-file `imports / declared` edges** alongside the
956 `defines`, attributed to syn. tree-sitter is now reached only for
languages that are neither Rust nor Python, which is where it belongs.

`imports` is `declared` rather than `structural` on the Phase 4 table's
own terms: a use statement is readable off the page, and syn read it.

Two follow-ups this leaves, both named rather than assumed away.
Python call edges resolve but this corpus has only 29 rustpython
symbols and produced none, so that path is wired and unexercised.
Rust *call* edges still need rust-analyzer through `lsp/edges.rs`,
which wants a live language server, and that is H8's managed toolchain.
Until then the Rust graph has import depth and no call depth.

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

1. **A warm run re-resolved every edge.** The `defines` pass filters on
   changed files and the cross-file resolvers do not, so a run where all
   63 files were skipped still upserted 213 import edges and reported
   them as written. They were upserts rather than new rows, but the
   summary read as though work happened. The edge pass now returns early
   when nothing changed at all, and a test asserts it. Any change
   anywhere still reopens the whole corpus, because a new symbol in one
   file can be the target of an import in a file that did not change.
2. **`ALTER TABLE ... ADD COLUMN IF NOT EXISTS` deadlocks a concurrent
   test suite.** It takes an ACCESS EXCLUSIVE lock *before* evaluating
   the IF NOT EXISTS, so every `apply_schema` fought readers, and three
   claims failed with SQLSTATE 40P01 the first time the new column
   landed. Both that ALTER and the `audit_log.outcome` one from spec
   010, which had the same latent hazard and had only been lucky, are
   now guarded by a catalog check inside a DO block. The lock is taken
   only when the column is actually missing.
3. **Skipped files still feed resolution.** A file whose hash matched
   contributes no writes but does contribute its symbols to the map,
   because an edge pointing into an unchanged file must still resolve.
   Getting this wrong would have made warm runs silently lose edges.
4. **Chunk-to-symbol linkage crosses a unit boundary.** Chunk offsets
   are bytes and symbol positions are lines, so the orchestrator
   converts rather than comparing across units.
5. **Embedding is opt-in, and the spec said the opposite.** EC-4 asked
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
