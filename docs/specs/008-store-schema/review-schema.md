# Review Notes: the store schema

Reviewer: Claude, with Todd. Date: 2026-08-13. The first construction of the
build era, executed against the sealed dev cluster.

## What exists now

`crates/yeomna-store`: `schema.sql` (128 lines, ruling citations inline),
an idempotent concurrency-safe applier (`pg_advisory_lock` serializes
appliers, since concurrent IF NOT EXISTS DDL races in the catalogs), and
the seven claims as integration tests that ran green against the cluster in
0.04 seconds and skip cleanly when no socket exists.

The schema is applied to the dev cluster. Eight objects: `graphs`, `nodes`,
`chunks`, `embeddings`, `edges` with its three basis partitions, `node_log`,
`audit_log`, and the `edge_basis` enum.

## The emitted-type mapping (spec SC5)

| Emitted | Lands as |
|---|---|
| `SymbolDocument.key`, `chunk_doc._key`, document keys | `nodes.natural_key` (UNIQUE per graph, golden-key upsert tested) |
| `SymbolDocument.kind`, document rows | `nodes.kind` CHECK (five primitives plus `document`) |
| every other `SymbolDocument`/`FileAnalysis`/metadata field | `nodes.payload` jsonb |
| `TextChunk.text/chunk_index/start_char/end_char` | `chunks` columns, byte offsets per the CRLF-exact contract |
| ontology `chunks.symbols[]` | `chunks.symbol_ids bigint[]`, GIN |
| `embedding_doc.embedding` | `embeddings.vec halfvec(2048)` |
| `EmbedResult.model` plus staleness | `embeddings.model`, `model_hash` |
| `CrateEdge.kind` | `edges.relation` CHECK (four code relations plus `contains`) |
| `CrateEdge` endpoint strings | resolved to `src_id`/`dst_id` by the sink (spec 009), transitional prefixes stripped |
| Q1's required provenance | `edges.basis` NOT NULL enum, `edges.analyzer` NOT NULL |
| `CrateEdge.metadata` | `edges.payload` |

## Findings from execution

1. **Concurrent schema application races.** Seven parallel appliers hit
   catalog races even with per-statement idempotence. The applier now takes
   an advisory lock, which is correct library behavior beyond the tests.
2. **Null basis is rejected by partition routing (23514) before NOT NULL
   (23502) fires.** Two mechanisms, same fact: an unattributed edge is
   unrepresentable. The claim test accepts both proofs.
3. **Driver type edges:** tokio-postgres refuses `&str` into enum or jsonb
   parameters, so the tests cast through text. The sink implementation
   (spec 009) should decide once whether to map `edge_basis` as a Rust enum
   with the postgres-types derive or keep text casts at the boundary.
4. The claims went from hand-probes to permanent tests exactly as specced:
   pruning through the recursive term, the halfvec/vector split (re-verified
   by psql during a test bug, the refusal is real), cascade completeness,
   golden-key upsert, append-only grants, and the pinned shapes landing with
   FTS answering over them.

## Verification

- Live: 7/7 claims green, schema applied idempotently (every test applies
  it again on entry).
- Absent cluster (`YEOMNA_TEST_DB=/nonexistent`): 7/7 skip-pass, G2 holds.
- Workspace gate green, clippy clean, fmt clean.
