# Review Notes: the ingest sink

Reviewer: Claude, with Todd. Date: 2026-08-13. The sink half of H1,
executed against the sealed dev cluster.

## What exists now

`PgSink` in `crates/yeomna-store/src/sink.rs` implements
`yeomna_pipeline::sink::IngestSink`, the first and only implementor of the
trait outside test mocks. The store crate now depends on the pipeline
crate, which is the dependency arrow D1 implies: the trait is the
pipeline's contract, the resolution is the sink's job.

Also landed: the edge identity ruling as `edges_identity` (unique on
`graph_id, src_id, dst_id, relation, basis`, applied idempotently over a
cluster that already ran 008's schema), claim 8 proving both the refusal
and the `ON CONFLICT` upsert path H3 will use, and seven sink integration
tests including the five-call sequence.

## The JSON-field-to-column mapping as built

| JSON field | Lands as |
|---|---|
| metadata `_key` | `nodes.natural_key`, container picks `kind` (`documents` to `document`, `codebase_files` to `file`) |
| every other metadata field | `nodes.payload` verbatim |
| chunk `doc_key` | resolved to `chunks.node_id`, memoized per batch |
| chunk `chunk_index`, `text`, `start_char`, `end_char` | columns |
| chunk `_key`, `total_chunks` | not stored, derivable (pinned key format, count) |
| embedding `chunk_key` | parsed to the index, validated by reconstruction through `yeomna_keys::chunk_key`, resolved to `embeddings.chunk_id` via `(node_id, chunk_index)` |
| embedding `embedding` | `embeddings.vec halfvec(2048)` |
| embedding model | not in the document: read from the parent node's `payload->>'embedding_model'`, written first per the five-call order, `model_hash` derived through `yeomna_keys` |

## Findings from execution

1. **The trait takes `&self`, so `Client::transaction` (which needs
   `&mut`) is out.** Transactions are explicit `BEGIN`, `SAVEPOINT`,
   `COMMIT` statements on the shared connection. The module doc states
   the consequence: one sink, one sequential caller, which is how the
   orchestrator drives it. Concurrent ingest stays M3's question.
2. **Savepoint isolation was proven by the wrong-dimension test.** A
   3-element vector into `halfvec(2048)` dies at the database mid-batch,
   rolls back to its savepoint, counts one error, and the batch's
   survivors commit. Database rejections (`as_db_error().is_some()`)
   count per document, transport failures abort the batch.
3. **The chunk-key parse survives a pathological parent.** A document
   key containing the separator (`a_chunk_9`) still round-trips, because
   validation reconstructs through the pinned format instead of trusting
   the split.
4. **`overwrite: false` duplicate detection rides `execute`'s affected
   count.** `ON CONFLICT DO NOTHING` returns 0 affected for an existing
   key, which counts as the error the reference's import semantics
   expect, and the stored row is proven unchanged.

## The M3 restatement (spec success criterion 5)

Each `insert_documents` batch is one READ COMMITTED transaction with a
savepoint per document. Each removal is a single statement. The
delete-then-insert window across sink calls survives from the reference
by design, and concurrent ingest of the same document is not defended.
Both belong to M3, named here, not resolved here.

## Divergence from the spec

None in behavior. One addition: `PgSink::graph_id()` as a read accessor,
used by nothing yet, kept because tests and H3 both want it and it
exposes no verb-layer surface.

## Verification

- Live, twice consecutively: 2 unit tests, 8 schema claims, 7 sink
  integration tests, all green both runs (cleanup and idempotent
  re-apply proven).
- Absent cluster: all skip-pass, G2 holds.
- Workspace gate 257 tests green, clippy clean, fmt clean, store-brand
  greps clean.
