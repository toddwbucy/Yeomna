# Review notes: 019 the ingestion verbs

Status: build complete 2026-09-09, local review run before the PR
opened. Rulings applied: R21 D1 (long-running verbs block), D2
(`drift` re-walks, reports, writes nothing), R17's precedent (a verb
needing its own connection opens one).

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## What landed

Four verbs: `codebase.ingest`, `ingest` (documents plus the `conforms`
links), `codebase.drift`, and `codebase.validate`. The Phase 6 refusal
list now names only `codebase.retire` and `codebase.prune`.

`drift` joined `yeomna-pipeline` rather than the verb layer, sharing
the orchestrator's own walk and analysis, because a drift that agreed
with a different analyzer than the one filling the graph would report
drift where there is none or miss it where there is. `IngestProbe`
grew one read, `stored_file_keys`, so the pipeline can see what the
graph holds without learning SQL.

## Build findings

**1. A latent defect the daemon's shape exposed: the ingest future was
not `Send`.** `walk_symbol` in the Go language-server path returns
`Pin<Box<dyn Future<Output = ()>>>` with no `Send` bound, which makes
every caller non-`Send` all the way up through `ingest_codebase` to
`Session::call`. Nothing noticed while ingest was only ever awaited
from a test, and the daemon spawning one task per connection is what
found it: the workspace stopped compiling the moment a verb reached
the orchestrator. The bound is added, and
`ingestion_verbs::a_call_is_spawnable` is a compile-time guard so it
cannot come back. Worth stating plainly: this was a real bug that
would have shipped, and what caught it was a consumer with a stricter
requirement rather than a test.

**2. Ingesting verbs open their own connection.** The session holds its
client inside the call lock (R16) and the orchestrators take a
`PgSink`, which owns a `Client`. Reshaping the session around one verb
was the alternative. The per-call connection is R17's pattern already
in the crate, and the session's audit row still brackets the whole
operation, so a crash-shaped ingest leaves the same NULL outcome any
other verb would.

**3. Ingest refuses to create a graph.** `PgSink::new` upserts the
graph, which is right for a library and wrong for a verb: a typo would
become a new graph and the caller would see a successful ingest into
nothing. The verb checks first and answers `NotFound` naming what to do.

**4. `validate` has real content, and it is one hole.** The schema
already enforces basis and analyzer presence, edge identity, node
kinds, relation shape per partition, and referential integrity. What it
cannot express is that an edge's endpoints belong to the same graph as
the edge, because the foreign keys point at `nodes(id)` and know
nothing about `graph_id`. A traversal over such an edge walks out of
its own graph. The test plants one and watches validate find it.

## The dogfood

Through the CLI, against the live cluster, before the PR opened:

```
$ yeomna call '{"verb":"codebase.validate","args":{"graph":"yeomna_self"}}'
  ok: true, cross_graph_edges: 0, chunks_naming_foreign_symbols: 0

$ yeomna call '{"verb":"codebase.drift","args":{"graph":"yeomna_self","path":"."}}'
  clean: false, seen: 92, unchanged: 67
  changed: 12   (the pipeline files this spec and 015 touched)
  new: 13       (the yeomna-cli and yeomna-daemon crates)
  missing: 0
```

The graph is clean and knows exactly how stale it is: the thirteen new
files are the two crates built today, and the twelve changed ones are
what specs 015 and 019 edited. That is the verification loop the graph
was built for, answering about the work that built it.

## Test inventory

`crates/yeomna-verbs/tests/ingestion_verbs.rs` (9): FR1 through FR7 and
EC-1 through EC-8. The two that matter most are drift proving it wrote
nothing (node, edge, and log counts identical across a drift that
reported changes) and validate finding a planted cross-graph edge.
`tests/audit.rs` grew four refusal-shaped entries so G1 stays total,
and `read_verbs.rs`'s shrinking guard now names Phase 6b alone.

## CodeRabbit round one

**A present file that could not be assessed was reported as missing,
and `missing` is what `retire` will act on.** `walk_and_analyze` skips
a file on three paths (over the size limit, unreadable, unparseable)
and none of them reached the list it returns, so drift's
`missing` (stored keys minus analyzed keys) included files that were
sitting right there in the tree. Phase 6b would then have swept their
nodes. The walk now reports what it skipped, drift counts those keys as
seen, and they land in a new `unassessed` list with the reason rather
than in `missing`. `clean` is false when anything went unassessed,
because a drift that could not read a file does not answer yes.

The new test covers both skip paths and would have failed before the
fix: `seen_keys` would have been empty, both present files would have
been reported missing, and `files_seen` would have read zero.

Also applied: the seeding ingests in three tests now assert their
envelopes, so a failed setup reports itself rather than surfacing as a
confusing assertion three lines later.

## Riding items

- D1's blocking rule has no test for a call that takes minutes, since
  nothing implemented does. The WeaverTools stand-up is the first
  ingest large enough to feel it.
- The language-server pass stays off from a verb. Exposing it as a
  request field is a ruling for when someone wants a minute-scale call.
- Document drift does not exist. `codebase.drift` is codebase-scoped by
  its name, and documents get theirs when a corpus asks.
