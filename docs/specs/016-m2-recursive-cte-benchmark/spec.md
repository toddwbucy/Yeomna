# Specification: 016 The M2 Benchmark, Recursive CTEs at Depth

Owner: charter section 10, M2, and the store PRD's Risk Assessment.
Ruled by R21 D8 (2026-09-09): report-only, measurement rather than
threshold. The second PR in R21's order.
Status: draft, 2026-09-09.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

M2 asks one question the charter left open on purpose: do hand-rolled
recursive CTEs stay clean on deep variable-length traversal of a real
code graph, where `calls` edges cycle? Partition pruning through the
recursive term was proven at the verb level in spec 013. Untested is
cycle behavior and row growth at depth on a real graph, and the
charter's instruction is to try to blow up the depth-20 ceiling rather
than confirm that depth 3 works.

The corpus exists (`yeomna_self`, the dogfood graph, 1276 `calls`
edges after the 2026-09-09 re-ingest, call chains reaching the depth
cap). The instrument exists (`graph.traverse`, D7's `UNION (node,
depth)` walk). What is missing is the run and the report, and the store
PRD names one more thing: the benchmark must test both formulations,
because the reference's path-enumerating shape is the one that
exploded on a synthetic graph (2.1 million rows, 430 MB spilled, at
depth 20 on 6000 edges with branching factor 2).

## Task Scope

- An `#[ignore]` benchmark in `yeomna-verbs`'s tests, run explicitly,
  against `yeomna_self`. It reports and asserts nothing about
  thresholds (D8).
- Corpus facts first: node and edge counts, `calls` edges, the hubs by
  out-degree, and cycle evidence (two-cycles on `calls`, and for each
  hub whether the walk reaches the hub again at depth above zero).
- The D7 walk through the verb, from each hub, over `calls` alone and
  over every relation, at depths 1, 2, 3, 5, 10, 20, 30, 50, and 100,
  with the row cap raised high enough that the cap is not what stops
  it. Recorded per run: nodes visited, whether the cap was hit, wall
  time, and temp bytes written by the cluster during the run (the
  spill signal, read from `pg_stat_database`).
- The reference's formulation, as raw SQL on the owner connection: a
  `UNION ALL` walk carrying a path array with the `<> ALL(path)` cycle
  guard, the shape the store PRD identified as the one that explodes.
  Same starts, depths 1 through 30, a row limit of five million and a
  statement timeout of sixty seconds so it cannot take the cluster
  with it. Recorded: rows enumerated, wall time, spill, and whether it
  stopped on the limit or the timeout.
- The report: `docs/measurements/M2-recursive-cte-at-depth.md` with
  the tables, the reading of them, and what would reopen M2. The
  store PRD's M2 section, the charter's M2 paragraph, and the ledger's
  M2 mentions point at it.

## Out of Scope

- Thresholds, pass or fail (D8).
- Changing the traversal SQL. If the numbers argue for a change, that
  is a finding for a later spec.
- M1, M3, M4.
- A synthetic graph. The charter wants the real one.

## Files to Modify

- `crates/yeomna-verbs/tests/m2_benchmark.rs` (new).
- `docs/measurements/M2-recursive-cte-at-depth.md` (new).
- `docs/PRD-postgres-store.md` (the M2 risk section), `README.md` (the
  M2 paragraph, one pointer line), `docs/holes.md` (the M2 mentions).

## Files to Reference

- `crates/yeomna-verbs/src/graph.rs`: `traverse_sql`, `MAX_DEPTH`.
- `crates/yeomna-verbs/tests/graph_verbs.rs`: the K15 dense-graph
  proof and the dogfood cycle test, the two prior M2 signals.
- `docs/PRD-postgres-store.md` Risk Assessment: the synthetic probe's
  numbers, which the real run is measured against.

## Functional Requirements

- **FR1** The benchmark runs only when asked (`--ignored`), skips with
  a cause when the cluster or `yeomna_self` is absent, and touches
  nothing: every statement it runs is a read.
- **FR2** Both formulations run from the same starts at the same
  depths, so the comparison is like for like.
- **FR3** The reference formulation is bounded by a row limit and a
  statement timeout, and the run records which bound stopped it.
- **FR4** Spill is measured, not inferred: temp bytes written during
  each run, from the cluster's own statistics.
- **FR5** Cycle presence on the real graph is shown, not assumed.
- **FR6** The report carries every number the run printed, the reading
  of them, and the reopen condition.

## Edge Cases

- **EC-1** A hub with no `calls` out-edges: the walk visits one node at
  every depth, which is a valid row of the table, not an error.
- **EC-2** The reference formulation times out at a shallow depth: the
  table records the timeout and the run continues to the next start.
- **EC-3** `pg_stat_database.temp_bytes` does not move: recorded as
  zero spill, which is the finding.

## Implementation Notes

DO:

- DO read the cap and depth constants from the same numbers the verb
  uses, so the report describes the shipped instrument.
- DO print the tables in a shape that pastes into the report.
- DO run each configuration once cold and take the second of two runs
  for the timing, so the number is the query and not the cache warming.

DON'T:

- DON'T add thresholds or assertions on the numbers.
- DON'T touch `graph.rs`.
- DON'T run the reference formulation without both bounds in place.

## Success Criteria

1. The benchmark runs to completion against `yeomna_self` and prints
   both tables.
2. The report exists, the PRD and charter point at it, and the ledger
   records M2 as measured.
3. The gate stays green (the benchmark is ignored by default).

## QA Acceptance Criteria

- `cargo test -p yeomna-verbs --test m2_benchmark -- --ignored --nocapture`
  completes with the cluster up.
- `cargo build`, `cargo test` (3x), `cargo clippy --all-targets`,
  `cargo fmt --check`, all clean.
