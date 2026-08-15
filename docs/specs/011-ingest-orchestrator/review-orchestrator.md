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
| Symbols | 956 structural, 1422 after enrichment | 0 |
| Edges | 2629 with the semantic pass | 0 |
| Chunks | 205 | 0 |

Nodes by kind after the semantic pass: 776 callable, 560 value, 130
type, 84 module, 63 file. Edges: 1550 `defines / declared`, 848
`calls / structural`, 214 `imports / declared`, 17 `implements /
structural`. Analyzers: rust-analyzer 2287, syn 313, rustpython 29.
Zero unresolved endpoints, zero failures.

The warm run is the ruling working: every file's `symbol_hash` matched,
nothing was rewritten, and the history stayed quiet.

## Corrected twice: the resolvers were wrong, then absent

The first run produced 955 edges, every one `defines`. The first draft
of these notes blamed an upstream capability gap. Todd asked why
tree-sitter was being used on Rust and Python, then, after the first
correction, asked whether rust-analyzer was wired at all. Both
questions found a real defect and both first answers were wrong.

**Round one: the fallback resolver was doing all the work.**
Tree-sitter never ran for *analysis*, and the analyzer counts always
said so: syn for Rust, rustpython for Python. But tree-sitter's
resolver was the only one wired for *edges*, and it is the fallback for
languages with nothing better. It reads a `calls` metadata field syn
does not write, so it correctly produced nothing on a Rust corpus.
`rust_imports::resolve_rust_imports` and
`python_calls::resolve_python_calls` sat unused beside it. The claim
that nothing populates `metadata["calls"]` for either language was
false for Python, and came from a grep piped through `head -8` that
truncated before reaching `python.rs:284`. Wiring each language to its
own resolver took the graph to 213 cross-file `imports` edges.

**Round two: rust-analyzer was never wired at all**, and the claim
that H8 blocked it was also wrong. `grep -rln "LspSession::" crates/`
returned nothing: no code in this workspace had ever started a language
server. The binary was installed the whole time at
`/usr/lib/rustup/bin/rust-analyzer`, `resolve_and_probe` falls back to
PATH, and `LspSession`, `RustAnalyzerSession`, `RustSymbolExtractor`,
and `LspEdgeResolver` were all sitting on main, complete and unused.
H8 is the `tools status` and `tools install` CLI commands, which is a
different thing from the library being usable.

Worth stating plainly, because it recurred: `analyze_with_fallback`'s
semantic tier for Rust *is* syn, by definition in `semantic_analyzer`.
rust-analyzer is not a higher tier of that path. It is a separate
whole-crate pass, and nothing was calling it.

### What the semantic pass changed

| Edges | Fallback only | Per-language | With rust-analyzer |
|---|---|---|---|
| `defines / declared` | 955 | 956 | 1550 |
| `imports / declared` | 0 | 213 | 214 |
| `calls / structural` | 0 | 0 | **848** |
| `implements / structural` | 0 | 0 | **17** |

2629 edges, 2287 of them attributed to rust-analyzer, across two
crates, in 26 seconds against 2.5 for the syn-only path. Zero
unresolved endpoints throughout.

**The enrichment protocol is now exercised for the first time.** Phase
7 says symbols are written twice in one run, structurally and then
semantically, converging through the same derived key. Measured on
this graph: 829 symbols carry an `update` entry in `node_log` from
being written by syn and then enriched by rust-analyzer, 593 more were
found only by rust-analyzer, and **zero natural keys are duplicated**.
The keys converge, which is what FR 2 asserted and nothing had yet
tested.

**The graph has depth.** A recursive walk over `calls` reaches the
depth-10 cap this query set, which is the first corpus M2 has ever
had. M2 stays open: having the corpus is not the same as running the
benchmark and reporting it.

### Why it is opt-in

`semantic_rust` defaults false. The pass puts an external process in
the ingest path, waits on a workspace index, and costs a minute-scale
run instead of a second-scale one. It degrades rather than failing: a
language server that will not start, will not index inside
`semantic_timeout`, or dies mid-crate leaves the structural graph
standing and logs what happened. An environment problem should not
cost a usable graph.

### Round three: Go and C++, wired the same day

Todd asked for both. They turned out to be different shapes of work.

**C++ needs no server at all.** libclang runs in process during
analysis, at Semantic tier, and records call sites with USRs and
`resolution: "semantic"` in symbol metadata. `cpp_edges::
resolve_cpp_calls` reads them off the symbols the structural pass
already produced, so it sits beside `rust_imports` and `python_calls`
on the free path with no flag and no timeout. Verified on a two-file
fixture: one `calls / structural` edge attributed to libclang, plus
its three `defines`.

**Go joins the language-server pass.** `GoplsSession` and
`GoSymbolExtractor` mirror the Rust pieces exactly and feed the same
`LspEdgeResolver`, so the pass now groups Rust by crate and Go by
module and merges both extractions. The flag generalized from
`semantic_rust` to `semantic_lsp` accordingly, and edges are
attributed per source file rather than by one label over the batch.

**gopls is not installed on this box**, which turned the Go fixture
into a test of the degradation contract instead. It degraded exactly
as designed, and the whole fallback chain proved itself in one run:
gopls absent, so `analyze_with_fallback` dropped Go analysis to
tree-sitter, which *does* populate `calls` metadata, so
`tree_sitter_edges` resolved the cross-file call that rust-analyzer
would have resolved semantically. One `calls / structural` edge,
attributed to tree-sitter, with the ingest never failing. That is the
fallback resolver doing the one job it exists for.

The test asserts both worlds: where gopls is present it requires the
pass to index, and where it is absent it requires zero units indexed
and a standing structural graph.

### The resolver map as it now stands

| Language | Symbols | Cross-file edges | Server |
|---|---|---|---|
| Rust | syn | `rust_imports`, plus `LspEdgeResolver` under the flag | rust-analyzer, optional |
| Python | rustpython AST | `python_calls` | none |
| C++ | libclang | `cpp_edges` | none |
| Go | gopls, else tree-sitter | `LspEdgeResolver` under the flag, else `tree_sitter_edges` | gopls, optional |
| anything else | tree-sitter | `tree_sitter_edges` | none |

One limitation left standing and named: the fallback resolver is
selected by *language* rather than by which analyzer actually ran, so
a Rust file that fell back to tree-sitter would have its edges missed.
`fallback_reason` records when that happens, and syn failing on valid
Rust is rare enough that selecting on the analyzer is a refinement
rather than a defect to fix here.

Python call edges remain wired and unexercised: only 29 rustpython
symbols exist in this corpus and none resolved.

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
