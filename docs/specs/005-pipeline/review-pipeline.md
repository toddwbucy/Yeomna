# Review Notes: yeomna-pipeline lift

Reviewer: Claude, with Todd. Date: 2026-08-11. The orchestrator was read in
full before the spec was written, which is where the trait design came from.

## Accounting

| Source (`crates/hades-core/src/pipeline/`) | Destination | Lines | Diff class |
|---|---|---|---|
| `mod.rs` (9) | `src/lib.rs` (20) | 20 | Rewritten: wiring for the two new modules, provenance |
| `orchestrator.rs` (638) | `src/orchestrator.rs` | 638 plus tests | The five edit classes below |

New files: `src/sink.rs` (the trait and outcome type, with design rationale
in docs) and `src/profile.rs` (transitional container triples, registry
refused a third time).

Edit classes in the move commit, all named by the spec:

1. Import rewrites to `yeomna_chunking`, `yeomna_keys`, `yeomna_embed`.
2. The sink hole: `ArangoPool` field and parameter became `S: IngestSink`,
   the `Database(ArangoError)` variant became
   `Sink(Box<dyn Error + Send + Sync>)`, and the five store call sites
   became trait calls with explicit error boxing.
3. `CollectionProfile::default_profile()` and `::get(name)` became the
   transitional consts.
4. Test adaptation from registry lookups to the consts.
5. Provenance notes.

## What the review found

**The trait held at two methods.** Nothing in the caller needed a third, and
the spec forbids growth. The store PRD's Phase 7 implements it, nothing else
may.

**Everything behavioral moved intact:** two-phase GPU batching, per-document
error isolation, the chunk-to-embedding count check as a hard error,
EmptyChunks, the dual-key document contract with its collision guard, and
the byte-stable `chunk_doc` and `embedding_doc` shapes the store schema will
be designed against.

**The stale-delete tradeoff comment** moved with every observation intact
and its close-the-window decision handed to the store PRD's M3 by name.

**One deferral, recorded in the spec amendment:** the five-call-sequence
test through `Pipeline` needs a constructible pipeline, and
`ExtractionClient::connect` is eager. It arrives with the sink
implementation PR, where a stub extractor exists.

## Verification

- 4 tests passing (3 moved and adapted, 1 mock-sink), clippy clean, fmt
  clean, full workspace green.
- `grep -rn 'Arango|AQL|HADES' crates/yeomna-pipeline/src`: zero matches
  after tightening.
- `grep -rn 'impl IngestSink' crates/`: exactly one match, the mock, under
  `#[cfg(test)]`.
