# M5. Hybrid fusion, and the vector index it does not use

Measured 2026-09-10 while building spec 023. Report-only, following D8's
precedent that a measurement is a measurement and not a threshold.

**The headline: `query --hybrid` is correct and does not use
`embeddings_hnsw`.** The ranking is exact, which brute force always is.
The cost is linear in the graph's embedding count, which is what the
charter's own comparison against the reference says it should not be.
Fixing it needs a schema change, so it is measured and named here rather
than smuggled into the spec that found it.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. SQL keeps its
syntax.

---

## What was measured

The corpus is `wt_embed`, the WeaverTools corpus ingested with embedding
on: 235 documents, 1,480 chunks, **1,275 of them embedded** and 205 not,
because five documents are over the embedder's context ceiling (PRD D5).
The cluster is the dev cluster, PostgreSQL 18.4 with pgvector 0.8.6, and
`EXPLAIN (ANALYZE, BUFFERS)` was run on the verb's own statement with
real parameter values.

## The plan, and the plan it is not

The vector half of the fusion, as the verb sends it:

```
Limit (rows=60)
  Buffers: shared hit=9544
  -> Sort (rows=60)  Sort Method: top-N heapsort  Memory: 27kB
     -> Nested Loop (rows=1275)
        -> Nested Loop (rows=1480)
           -> Nested Loop (rows=235)
              -> Seq Scan on graphs g
              -> Bitmap Index Scan on nodes_graph_kind
           -> Index Scan using chunks_node_id_chunk_index_key on chunks c
        -> Index Scan using embeddings_pkey on embeddings e  (loops=1480)
Execution Time: 4.833 ms
```

The same nearest search with no graph filter:

```
Limit (rows=50)
  Buffers: shared hit=368
  -> Index Scan using embeddings_hnsw on embeddings
Execution Time: 0.280 ms
```

**26 times the buffers and 17 times the time, at 1,275 vectors.** Every
embedding in the graph is fetched, its distance computed, and the set
sorted. `halfvec(2048)` is 4 KB, so every vector is TOASTed and every
fetch is a detoast: measured at about 2.9 buffers per embedded chunk.

## Why the index is unreachable, and it is structural

Four shapes were tried against the live cluster.

| Shape | Result |
|---|---|
| `row_number() OVER (ORDER BY vec <=> q)` with the graph join | Nested Loop over all 1,275, full quicksort |
| `ORDER BY vec <=> q LIMIT n` inside a subquery, graph joined | Nested Loop over all 1,275, top-N heapsort |
| `WHERE EXISTS (graph subquery) ORDER BY vec <=> q LIMIT n` | HashAggregate then Nested Loop, no vector index |
| The same with `enable_seqscan`, `enable_nestloop`, `enable_hashjoin` all off | Merge Semi Join on `embeddings_pkey`, then a sort. Still no vector index |

The cause is that `embeddings` carries no column the graph filter can be
expressed in. The filter reaches it only through
`chunks -> nodes -> graphs`, so the planner drives from the graph side and
`embeddings` is on the inner side of a join, where an index cannot supply
the ordering the query wants. The last row above is the important one: it
is not a cost estimate the planner got wrong, because forcing every
alternative off still does not produce a vector index scan.

**The subquery shape was kept anyway**, because it turns a full quicksort
into a top-N heapsort (88 kB against 27 kB of sort memory) and because it
is the shape an index scan would need if the filter ever reaches it.

## A second finding, which the fix will need

`hnsw.ef_search` defaults to 40, and an HNSW scan returns at most that
many rows. Measured: forcing the index with `enable_seqscan = off` and
asking for `LIMIT 50` returned **40 rows**, silently. With
`SET LOCAL hnsw.ef_search = 200` and
`SET LOCAL hnsw.iterative_scan = strict_order` the same query returned all
50.

Nothing in the repository sets either. So a shape that does reach the
index would silently truncate the fusion's candidate depth to 40 unless
both are set, which means the fix is two changes and not one. The settings
were written, measured, and taken back out of the verb, because a setting
that only matters to a plan the query cannot produce is a comment claiming
something the code does not do.

## What the fix looks like

`embeddings` gains a `graph_id`, denormalized from the chunk's node and
`NOT NULL`, so the vector CTE becomes a single-table
`WHERE graph_id = $1 ORDER BY vec <=> $2 LIMIT $3`. That is pgvector's own
documented shape for filtered vector search, and with
`hnsw.iterative_scan` the index keeps pulling until the limit is satisfied
after the filter.

It costs a schema version, a drop, a re-apply, a template re-stamp, and a
re-ingest, which under T4 is what a schema change costs here. It also
makes `corpus_cohort` and `codebase.validate`'s cohort check
single-table reads instead of three-table joins.

Two things to settle when it is specced. Whether `chunks` gains the same
column, since it has the same indirection and no index that needs it.
And what `hnsw.ef_search` should be, given that the fusion's candidate
depth is capped at 1,000 and pgvector's ceiling is the same.

## What this does not say

It does not say hybrid retrieval is too slow. At this corpus size it is
4.8 ms, and the whole verb including the query's own forward pass through
the embedder is dominated by the GPU.

It does not say the ranking is wrong. An exhaustive scan is exact, and
HNSW is approximate, so the current path is the more accurate of the two.

It is a statement about how the cost grows. The next thing on the order
is the WeaverTools KG stand-up, and a graph of 100,000 embedded chunks
would make one `limit: 5` hybrid query touch on the order of 290,000
buffers and detoast several hundred megabytes. The charter's Phase 1
measurement is what made `halfvec` mandatory, on the grounds that
`vector(2048)` refuses an HNSW index. An index nothing reaches is the same
outcome by a different route, and worth saying out loud.

## Reproducing it

The verb's statement is in `crates/yeomna-verbs/src/read.rs`, in `hybrid`.
Take a vector out of the graph to use as the query:

```sql
SELECT e.vec::text FROM embeddings e
  JOIN chunks c ON c.id = e.chunk_id
  JOIN nodes n ON n.id = c.node_id
  JOIN graphs g ON g.id = n.graph_id
 WHERE g.name = 'wt_embed' LIMIT 1;
```

then run the vector CTE alone under `EXPLAIN (ANALYZE, BUFFERS)`, with and
without the `g.name` predicate.
