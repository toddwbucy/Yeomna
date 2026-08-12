# Specification: 005 Pipeline and the Sink Trait

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 5.
Status: draft, 2026-08-11.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

The last Rust lift, and the one that is deliberately part construction:
`yeomna-pipeline` from `pipeline/` (647 lines), with the orchestrator's four
store coupling points replaced by the `IngestSink` trait, defined here and
implemented nowhere. When this merges, the pipeline PRD is complete and the
missing store is a typed hole the store PRD's Phase 7 fills.

The PRD left the trait's method set unfixed until it had a real caller. The
caller has been read in full, and its entire store surface is two operations
and one outcome type. The trait is exactly that surface and nothing more.

## The trait, designed from the caller

The orchestrator touches the store at five call sites through two functions:

| Reference call | Sites | What it does |
|---|---|---|
| `crud::insert_documents(&db, container, &docs, overwrite)` | 3 (metadata, chunks, embeddings) | Batch upsert of JSON documents into a named container, returning created and error counts |
| `query::remove_docs_by_fields(&db, container, &fields, key)` | 2 (stale chunks, stale embeddings) | Delete documents where any named field equals the key |

Therefore:

```rust
/// The pipeline's write boundary. Defined by its one caller, implemented by
/// the store crate when one exists (store PRD Phase 7).
pub trait IngestSink: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Batch-upsert JSON documents into a named container.
    async fn insert_documents(
        &self,
        container: &str,
        documents: &[serde_json::Value],
        overwrite: bool,
    ) -> Result<InsertOutcome, Self::Error>;

    /// Remove documents where any of `fields` equals `key`.
    async fn remove_documents_by_fields(
        &self,
        container: &str,
        fields: &[&str],
        key: &str,
    ) -> Result<(), Self::Error>;
}

/// Per-batch outcome of an insert.
pub struct InsertOutcome {
    pub created: usize,
    pub errors: usize,
}
```

Decisions inside that shape, each from the caller's observed needs:

- **`Pipeline<S: IngestSink>` is generic, not dyn.** Native async trait
  methods are not dyn-compatible, and the pipeline never needs runtime sink
  swapping.
- **`PipelineError::Database(ArangoError)` becomes
  `Sink(Box<dyn Error + Send + Sync>)`**, keeping the source chain without
  naming any store.
- **The names say what happens, in no store's vocabulary.** Insert, remove,
  container, overwrite. The overwrite flag is load-bearing: it is the
  idempotent-reingest switch, and deterministic keys are what make it safe.
- **The trait must not grow speculative methods.** No transactions, no
  queries, no schema operations. The store PRD owns whether inserts get a
  transactional envelope (its M3 decision), and the verb layer owns reads.

## Transitional profile plumbing, ruled like containers.rs

The orchestrator names its containers through the reference's
`CollectionProfile` (`metadata`, `chunks`, `embeddings`, `foreign_key`), whose
308-line registry was refused in Phase 1 and again in Phase 4. The lift
carries a minimal transitional form inside `yeomna-pipeline`: the four-field
struct and two consts (the default and codebase triples), no registry, no
`get(name)`, no environment lookup. Same rules as `yeomna-code`'s
`containers.rs`: documented as transitional, forbidden from growing.

The three moved unit tests use the registry (`CollectionProfile::get`) and are
adapted to the consts, a counted edit class.

## Task Scope

### This Task Will

1. Create `crates/yeomna-pipeline` from `pipeline/` (`mod.rs` to `lib.rs`,
   `orchestrator.rs`).
2. Define `IngestSink` and `InsertOutcome`, make `Pipeline` generic over the
   sink, and rewrite the five store call sites to trait calls.
3. Carry the transitional profile struct and consts.
4. Move the three unit tests, adapted to the consts, and add a mock-sink test
   proving the trait is implementable and the hole is exactly sink-shaped.
5. Leave build, test, clippy, fmt clean at merge.

### Out of Scope

- **Any sink implementation.** The store PRD's Phase 7.
- **The Python services.** Phase 6.
- **Changing the pipeline's flow, batching, or error isolation.** The
  two-phase GPU ordering, the chunk-to-embedding count check, EmptyChunks,
  and the dual-key document contract move as they are.
- **Closing the delete-then-insert atomicity window.** The orchestrator's own
  comment documents the non-transactional stale-delete as a deliberate
  tradeoff that self-heals. That comment is load-bearing for the store PRD's
  M3 decision and moves intact, ArangoDB vocabulary in it corrected in
  tightening without weakening the observation.

## Allowed Edits in the Move Commit

1. `mod.rs` to `lib.rs`, module wiring, provenance note.
2. Import rewrites: `crate::chunking` to `yeomna_chunking`, `crate::db::keys`
   to `yeomna_keys`, `crate::persephone::` to `yeomna_embed::`.
3. **The sink hole itself**: `ArangoPool` field and parameter to `S:
   IngestSink`, the error variant, and the five call sites to trait calls.
   This is the PRD's named construction inside the lift, not drift.
4. The transitional profile struct and consts, replacing
   `db::collections::CollectionProfile` imports.
5. Test adaptation from the registry to the consts.

## Requirements

**FR-P1.** Public API otherwise unchanged: `Pipeline`, `PipelineConfig`,
`PipelineError`, `DocumentResult`, `PipelineSummary`, `process_document`,
`process_batch`, under the same names and semantics.

**FR-P2.** No implementor of `IngestSink` exists anywhere in the workspace.
The mock in tests lives under `#[cfg(test)]` and does not count.

**FR-P3.** The dual-key document contract (`doc_key` plus the profile's
declared foreign key, issue #165 in the reference) moves intact with its
tests, including the structural-field collision guard.

**FR-P4.** Document construction is byte-stable: `chunk_doc` and
`embedding_doc` emit the same JSON they emitted in the reference, since those
shapes are what the store PRD designs tables against.

**FR-P5.** Store-reference grep clean at merge except the load-bearing
tradeoff comment, corrected in tightening to name the behavior without the
store.

### Edge Cases

**EC-1.** Embedding count mismatch stays a hard per-document error, never a
truncation.

**EC-2.** `overwrite: false` skips the stale-delete, as in the reference.

## Implementation Notes

### DO

- Keep the trait in its own module (`sink.rs`) with the design rationale in
  its docs, since this trait is the contract the store crate implements.
- Add the mock-sink test: an in-memory recorder implementing `IngestSink`,
  asserting the five-call shape for one stored document (delete twice,
  insert three times, in that order).
- Keep per-file review notes.

### DON'T

- **Do not add trait methods the caller does not call.**
- **Do not implement the sink**, even a toy one outside tests.
- **Do not fix the atomicity window.** Recorded, deliberate, M3's problem.
- **Do not grow the profile plumbing.**

## Development Environment

```bash
cd /home/todd/git/Yeomna
cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check
```

Nothing requires Postgres or any service. The mock-sink test is the only
executable proof this phase can honestly offer, and that is the point: the
pipeline compiles against a store that does not exist yet.

## Success Criteria

1. Build, test, clippy, fmt clean from the repository root at merge.
2. `IngestSink` has exactly two methods and one outcome type.
3. No implementor outside `#[cfg(test)]`, checked by grep.
4. The three moved tests pass adapted, the mock-sink test passes, and
   document JSON shapes are pinned unchanged.
5. Store-reference grep clean at merge per FR-P5.
6. Review notes account for every edit class.

## QA Acceptance Criteria

1. The Issue and PR follow the established pattern.
2. The PR body states the handoff explicitly: the store PRD's Phase 7
   implements this trait, and nothing else in the workspace may.
