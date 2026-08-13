# Specification: 009 The Ingest Sink

Parent PRD: `docs/PRD-postgres-store.md`, Phase 7, against the schema spec
008 built. Fills the sink half of holes-ledger H1 and retires the spec 005
five-call deferral.
Status: draft, 2026-08-13.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. SQL and Rust keep
their syntax.

---

## Overview

The store crate implements `IngestSink`, the pipeline's write boundary.
After this spec, 12.3k lines of ingest machinery can write to Postgres
through the two methods the trait carries, and nothing else in the
workspace may implement it.

The trait was designed from its one caller (spec 005) and the schema was
designed from the emitted types (spec 008). This spec is the join: JSON
documents addressed by transitional container names, landing in the tables
those names stop meaning anything against.

Three decisions recorded during 008 land here: edge identity, the
`edge_basis` Rust mapping, and the five-call-sequence test.

## Task Scope

- `PgSink` in `crates/yeomna-store/src/sink.rs`, implementing
  `yeomna_pipeline::sink::IngestSink`.
- The edge identity ruling, applied as one unique index in `schema.sql`
  and one new claim test.
- The five-call-sequence integration test, store side.
- Review notes alongside this spec.

## Out of Scope

- **The ingest orchestrator (H3).** This spec makes the sink real. Nothing
  drives it end to end until H3.
- **Any edge-writing API.** The sink's callers never write edges (edge
  writing was `codebase_ingest.rs` work, which is H3). Only the identity
  index lands now, because it is schema and its evidence is in hand.
- **The `edge_basis` Rust mapping.** Deferred to H3 with rationale below.
- **Closing the stale-delete atomicity window.** The delete-then-insert
  sequence spans sink calls. M3 owns the cross-call window and this spec
  restates it without fixing it.
- **Verbs, RLS, connection pooling, config layer (H10), Python.**
- **Making `Pipeline` generic over extractor and embedder.** The 005
  deferral assumed a stub extractor would exist here. It does not, and
  parameterizing a merged crate's clients is H3-era construction, where
  the orchestrator is rewired anyway.

## Files to Modify

- `crates/yeomna-store/src/sink.rs` (new): `PgSink` and the trait impl.
- `crates/yeomna-store/src/lib.rs`: module wiring, error variants.
- `crates/yeomna-store/schema.sql`: the `edges_identity` unique index.
- `crates/yeomna-store/Cargo.toml`: `yeomna-pipeline`, `yeomna-keys`,
  `serde_json` become real dependencies.
- `crates/yeomna-store/tests/schema_claims.rs`: claim 8.
- `crates/yeomna-store/tests/sink.rs` (new): the sink integration tests.

## Files to Reference

- `crates/yeomna-pipeline/src/sink.rs`: the trait, its semantics doc.
- `crates/yeomna-pipeline/src/orchestrator.rs`: `store()` and
  `delete_doc_chunks()`, the five call sites, `chunk_doc` and
  `embedding_doc` construction.
- `crates/yeomna-pipeline/src/profile.rs`: the six container names and two
  foreign-key field names, the complete vocabulary the sink must speak.
- `crates/yeomna-keys/src/lib.rs`: `chunk_key`, `embedding_key`,
  `model_hash`, the golden values.
- `docs/specs/008-store-schema/spec.md` and its review notes: the tables,
  the recorded decisions this spec inherits.
- `docs/PRD-postgres-store.md`: Phase 7, D1, D2, M3.

## Resolved here: edge identity (recorded in 008 review notes)

**Ruling: an edge is identified by `(graph_id, src_id, dst_id, relation,
basis)`. `analyzer`, `status`, and `payload` are attributes of the edge,
not parts of its identity.**

Evidence, measured 2026-08-13: every analyzer in `yeomna-code` already
deduplicates at emission on exactly this triple of endpoints and relation.
`tree_sitter_edges.rs` keeps `seen_calls` and `seen_imports` sets,
`cpp_edges.rs` keeps `seen`, `lsp/edges.rs` dedups on
`(String, String, &str)`, `python_calls.rs` documents "the deduplicated
edge list", and `rust_imports.rs` pins it with a test. The constraint
encodes a promise the emitters already make.

Applied as:

```sql
CREATE UNIQUE INDEX IF NOT EXISTS edges_identity
    ON edges (graph_id, src_id, dst_id, relation, basis);
```

`basis` is in the key, which LIST partitioning requires. When H3 writes
edges, `ON CONFLICT` on this index updates `analyzer`, `status`, and
`payload` in place: last writer wins, matching node upsert semantics.

## Deferred here: the `edge_basis` Rust mapping

The sink writes no edges, so a typed `edge_basis` on the Rust side has no
consumer in this spec. Per G4, the mapping (postgres-types derive versus
text casts at the boundary) is decided in H3's spec, where edge writes
exist. The claim tests keep their text casts.

## The design

### Construction

`PgSink::new(client, graph_name)` resolves or creates the graph row and
holds `graph_id` for its lifetime. One sink, one graph: multi-graph
ingest is one sink per graph. The sink connects as `yeomna_app`, the
runtime role. `apply_schema` stays owner work.

### Container mapping

The six names from `profile.rs`, closed:

| Container | Table | Node kind |
|---|---|---|
| `documents` | `nodes` | `document` |
| `codebase_files` | `nodes` | `file` |
| `chunks`, `codebase_chunks` | `chunks` | |
| `embeddings`, `codebase_embeddings` | `embeddings` | |

An unknown container is a caller bug and returns `Err`, never a counted
per-document error. The collection prefix inside any endpoint string is
stripped by the sink when it resolves (D1), though no current document
carries one.

### Document mapping

**Metadata documents** (`_key`, `full_text`, counts, `embedding_model`,
`embedding_dimension`, `extractor_metadata`, foreign key): `_key` becomes
`natural_key`, the container picks `kind`, everything else lands in
`payload` verbatim. Upsert per Phase 7:
`INSERT ... ON CONFLICT (graph_id, natural_key) DO UPDATE SET payload`.

**Chunk documents** (`_key`, `doc_key`, `text`, `chunk_index`,
`total_chunks`, `start_char`, `end_char`, foreign key): the parent node is
resolved once per batch from `doc_key`. Columns take `chunk_index`,
`text`, `start_char`, `end_char`. Two fields are derivable and not
stored: `_key` (the pinned `{doc_key}_chunk_{i}` format) and
`total_chunks` (a count over the document's chunks). `symbol_ids` stays
empty until H3. Upsert on `(node_id, chunk_index)`.

**Embedding documents** (`_key`, `chunk_key`, `doc_key`, `embedding`):
the chunk is resolved by `(node_id, chunk_index)`, with the index parsed
from `chunk_key`'s pinned suffix and validated by reconstructing the key
through `yeomna_keys::chunk_key` and comparing. The vector lands as
`halfvec(2048)`. The document carries no model name, so the sink reads
`embedding_model` from the parent node's payload, which the five-call
order guarantees was written first, and derives `model_hash` through
`yeomna_keys::model_hash`. Upsert on `chunk_id`.

### Overwrite semantics

`overwrite: true` upserts (`DO UPDATE`). `overwrite: false` inserts with
`DO NOTHING`, and a document whose key already existed counts in
`InsertOutcome::errors`, matching the reference's import semantics that
the orchestrator checks per batch. EC-2 of spec 005 (no stale-delete
without overwrite) stays orchestrator behavior and needs nothing here.

### Removal semantics

`remove_documents_by_fields(container, fields, key)`: in both registered
profiles, every field name passed (`doc_key`, `parent_key`, `file_key`)
denotes the parent document key, so the operation is "remove this
document's rows from this container." The sink resolves the parent node
and deletes `chunks` rows by `node_id`, or `embeddings` rows through the
chunk join. A field name outside the known parent-key set is `Err`: a
sink that guessed would delete the wrong rows silently. A missing parent
node is a no-op `Ok`, because removal is idempotent and a first-run
overwrite legitimately deletes nothing.

### Transactions, and the M3 statement Phase 7 requires

Each `insert_documents` batch is one transaction with a savepoint per
document. A failing document rolls back to its savepoint and increments
`errors`, the survivors commit. Each removal call is a single statement in
its own transaction. Isolation is READ COMMITTED, named explicitly:
within a batch it is sufficient because the batch touches one document
key's rows. Across calls, the delete-then-insert window from the
reference survives by design, and concurrent ingest of the same document
is not defended here. Both belong to M3 and this spec restates them
without resolving them.

## Functional Requirements

1. `PgSink` implements `IngestSink` with `Error: std::error::Error + Send
   + Sync + 'static`.
2. All six container names route correctly, unknown names are `Err`.
3. Re-running an identical batch with `overwrite: true` yields the same
   row count (idempotent reingest, the trait's load-bearing flag).
4. `overwrite: false` on an existing key counts an error and leaves the
   stored row unchanged.
5. A rerun producing fewer chunks, preceded by the orchestrator's two
   removal calls, leaves no orphaned chunks or embeddings.
6. Malformed documents (missing `_key`, non-string keys, missing
   required fields, wrong vector dimension) count as per-document errors
   without failing the batch.
7. The edge identity index exists and a duplicate edge insert violates
   it (claim 8).

## Edge Cases

- **EC-1.** An embedding batch whose parent node lacks `embedding_model`
  in payload: per-document error, not `Err`. The metadata write is the
  orchestrator's responsibility and its absence is data corruption worth
  surfacing per document.
- **EC-2.** `chunk_key` that fails reconstruction against
  `yeomna_keys::chunk_key`: per-document error. Never trust a parsed
  index that does not round-trip.
- **EC-3.** Empty `documents` slice: `Ok` with zeros, no transaction.
- **EC-4.** Removal against a container that is not `chunks`-like or
  `embeddings`-like (the caller never does this today): `Err`.

## Implementation Notes

### DO

- Depend on `yeomna-pipeline` from `yeomna-store`. The trait is the
  pipeline's, the implementation is the store's, and the dependency
  arrow matches D1's "the sink owns resolution."
- Resolve the parent node once per batch, not per document.
- Keep the JSON-to-column mapping in one place with the 008 review
  notes' emitted-type table as its comment anchor.
- Reuse the schema-claims skip pattern for the new integration tests.
- Test the chunk-key parse against the golden values in `yeomna-keys`.

### DON'T

- Do not add methods to the trait, and do not add public API to
  `PgSink` beyond construction. The verb layer is not this.
- Do not implement edge writes, symbol containers, or a second profile
  registry.
- Do not close the M3 window or serialize concurrent ingest.
- Do not store derivable fields (`_key` of chunks, `total_chunks`).
- Do not consult the closed reference. The contracts named above are
  complete.

## Success Criteria

1. Workspace gate green with no cluster (new tests skip, stated).
2. Against the dev cluster: all functional requirements demonstrated by
   integration tests, including the five-call sequence in the
   orchestrator's exact order with a fewer-chunks rerun.
3. Claim 8 (edge identity) passes and the index applies idempotently to
   a cluster that already ran spec 008's schema.
4. Store-brand greps stay clean.
5. Review notes record the JSON-field-to-column mapping as built, any
   divergence from this spec, and the M3 restatement.

## QA Acceptance Criteria

1. `cargo test -p yeomna-store` twice in a row against the live cluster:
   second run green proves the tests clean up and the schema re-applies.
2. `cargo test --workspace`, `cargo clippy --all-targets`,
   `cargo fmt --check` from the root, all clean.
3. Editorial sweep of spec and review notes clean.
4. Issue plus draft PR per the standing workflow, fixes as follow-up
   commits.
