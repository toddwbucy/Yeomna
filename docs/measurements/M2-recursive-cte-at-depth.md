# M2. Recursive CTEs at depth on the real code graph

Measured 2026-09-09 under spec 016, report-only per R21 D8. The
instrument is `graph.traverse` (D7's `UNION (node, depth)` walk, spec
013) and the reference's path-enumerating shape run as raw SQL under a
row limit and a statement timeout. The corpus is the dogfood graph,
which the charter named as the one M2 needed, with the WeaverTools
census graph as a second, call-sparse data point. The benchmark is
`crates/yeomna-verbs/tests/m2_benchmark.rs`, run with `--ignored`, and
`YEOMNA_M2_GRAPH` selects the graph.

**How spill is measured, and how far to trust it.** The signal is
`pg_stat_database.temp_bytes`, read as a difference. Two limits are
worth stating. The counter is database-wide, so it holds for a cluster
with one caller, which the dev cluster is. And
`pg_stat_force_next_flush()` flushes only the calling backend, so a
per-query read on the owner connection cannot make the `yeomna_app`
backend that ran a D7 walk publish its pending statistics yet.

So the run reports three things rather than one. The **instrument
check** runs first: a sort forced past a 64kB `work_mem`, which spilled
**17,498,112 bytes**, so the counter is live and a zero means no spill
rather than no measurement. The **per-query columns** below are
indicative under the flush caveat. And the **total across the whole
run**, taken after every backend has finished and settled, is the
authoritative number: **0 bytes**.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## The question, as the charter put it

Do hand-rolled recursive CTEs stay clean on deep variable-length
traversal of a real code graph, where `calls` edges cycle? Try to blow
up the depth-20 ceiling rather than confirm that depth 3 works. Test
both formulations, because the reference's shape exploded on a
synthetic graph: 2.1 million rows and 430 MB spilled at depth 20 on
6000 edges with branching factor 2 (store PRD, Risk Assessment).

## Run one: the dogfood graph (`yeomna_self`)

| | |
|---|---|
| Corpus | 1976 nodes, 3533 edges, 1325 `calls`, **one** two-cycle on `calls` |
| Hubs by `calls` out-degree | `Session::dispatch` (31), `semantic_lsp_pass` (18), `write_edges` (14) |
| Cycle evidence | no hub returns to itself within depth 20 |

D7 walk through the verb, cap one million so the cap never stops it,
second of two runs timed:

| Start | Reachable, saturates by | Depth 1 | Depth 5 | Depth 20 | Depth 100 | Spill |
|---|---|---|---|---|---|---|
| `Session::dispatch` | 57 nodes, depth 3 | 32 in 4 ms | 57 in 3 ms | 57 in 3 ms | 57 in 3 ms | 0 |
| `semantic_lsp_pass` | 76 nodes, depth 5 | 19 in 3 ms | 76 in 3 ms | 76 in 4 ms | 76 in 12 ms | 0 |
| `write_edges` | 29 nodes, depth 3 | 15 in 3 ms | 29 in 3 ms | 29 in 3 ms | 29 in 4 ms | 0 |

The all-relations walk from the same starts is identical, because a
callable's outgoing edges are `calls` alone (`defines`, `imports`, and
`contains` run file to symbol, never out of a symbol).

The reference shape, `UNION ALL` with a path array and the `<> ALL`
cycle guard, five million row limit, sixty second timeout:

| Start | Paths, saturates by | Depth 20 | Depth 30 | Spill | Stopped by |
|---|---|---|---|---|---|
| `Session::dispatch` | 190, depth 5 | 190 in 0 ms | 190 in 0 ms | 0 | exhausted |
| `semantic_lsp_pass` | 185, depth 10 | 185 in 0 ms | 185 in 0 ms | 0 | exhausted |
| `write_edges` | 57, depth 3 | 57 in 0 ms | 57 in 0 ms | 0 | exhausted |

## Run two: the WeaverTools census graph (`weavertools_census`)

| | |
|---|---|
| Corpus | 3593 nodes, 5238 edges, 240 `calls`, zero two-cycles |
| Caveat | the census ingest ran without the semantic pass, so the Rust call graph is unresolved and the hubs are Python experiment scripts at 13 out-calls each |

D7: reachability 22, 29, and 17 nodes, saturating by depth 5, two to
three milliseconds at every depth through 100, zero spill. The
reference shape: 32, 43, and 18 paths, exhausted, sub-millisecond,
zero spill.

## The reading

**Neither formulation blows up on a real code graph, and the reason is
the graph.** The synthetic explosion needed branching factor 2 sustained
across depth with dense diamonds and cycles. A real call graph is
nearly a forest: one two-cycle among 1325 `calls` edges, the largest
hub's reachable set 76 nodes, saturation by depth 5 from every hub
tried. Below saturation there is nothing for depth to multiply, and
the depth-20 ceiling is a number the real graph never approaches.

**D7's cost past saturation is the depth loop itself, not rows.** The
verb clamps depth at 100, and from `semantic_lsp_pass` the walk costs
3 ms at depth 5 and 12 ms at depth 100 with the visited set unchanged,
because `UNION` on `(node, depth)` keeps producing the same nodes at
new depths until the depth bound stops it. Twelve milliseconds is the
whole of the penalty for asking a question a hundred deep, and zero
bytes spilled.

**The reference shape is safe here and was shown unsafe elsewhere.**
Spec 013's K15 test (a complete graph on fifteen nodes) is the
adversarial corpus, where path enumeration is factorial and D7's
`UNION` dedup keeps the walk polynomial with the cap bounding it. This
run shows the same reference shape is harmless on the graph an
appliance will meet in practice. Both facts hold, and together they
say the shipped choice was right for the wrong reason the charter
feared: D7 does not protect against the real graph, it protects
against the pathological one, and the real graph did not need
protecting.

**What the numbers do not say.** A corpus whose call graph carries
real cycle density, mutual recursion clusters or event loops with
reachable sets in the thousands, could still move these tables. The
WeaverTools graph with its semantic pass run (PR 5 in R21's order,
which resolves its Rust `calls` through rust-analyzer) is the next
such corpus, and the benchmark re-runs against it in one second.

## What the local review left standing

The benchmark measures three hubs by `calls` out-degree, which is where
a walk has the most room to grow, and not the whole graph. A corpus
whose growth lives somewhere other than its busiest callers would need
its starts chosen differently, and the instrument takes that as an
argument rather than a rewrite. The second run's starts are Python
experiment scripts because the census ingest ran without the semantic
pass, which is the caveat above and the reason PR 5 in R21's order is
the better second corpus.

## Disposition

M2 is measured. The charter's open question has an answer on the
corpus the charter named: hand-rolled recursive CTEs stay clean at
depth 100 on the real graph, with zero spill, and the row cap and depth
clamp from spec 013 stay as the guards for graphs that are not this
one. No change to the traversal SQL follows from this run.

**Reopens when** a real corpus shows a hub whose reachable set does not
saturate by depth 20, or any spill at all on the D7 walk. The
benchmark is the instrument for saying so.
