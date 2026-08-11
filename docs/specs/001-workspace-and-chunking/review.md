# Review Notes: 001 Workspace and Chunking

Reviewer: Claude, with Todd. Date: 2026-08-10. Every moved file was read in
full before the move, per the spec.

## Per-file accounting

| Source (HADES-Burn `crates/hades-core/src/chunking/`) | Destination | Lines | Diff against source |
|---|---|---|---|
| `mod.rs` | `crates/yeomna-chunking/src/lib.rs` | 39 -> 44 | Provenance note added to crate docs. Nothing else. |
| `strategies.rs` | `crates/yeomna-chunking/src/strategies.rs` | 485 | Byte-identical, confirmed by `diff`. |
| `late.rs` | `crates/yeomna-chunking/src/late.rs` | 209 | Byte-identical, confirmed by `diff`. |

733 source lines moved, matching the PRD's measured figure for `chunking/`.

## What the review found

**No store coupling.** Zero references to any database. The one external
mention is a doc comment in `strategies.rs` pointing at the Persephone service
for BPE-accurate token counts, correctly describing the boundary: this crate
computes fast whitespace approximations and the service owns real tokenization.

**No dependencies.** The module uses std alone, and the crate manifest has an
empty `[dependencies]` section to keep it that way visibly.

**Behavior notes, recorded not fixed, per R1:**

1. `TokenChunking` with `overlap >= chunk_size` degrades step to 1, producing
   maximal overlap rather than an error. Deliberate-looking (`.max(1)`), noted
   as a place a caller could surprise themselves.
2. `SentenceChunking` merges a trailing chunk smaller than `min_chunk_size`
   into the previous chunk, so `min_chunk_size` is only enforced at the tail,
   not between chunks. Matches the field docs, worth knowing when tuning.
3. `split_sentences` is byte-oriented and treats only ASCII `.` `!` `?` as
   sentence enders. Unicode terminators (CJK full stop, for instance) do not
   split. The Unicode tests cover offset correctness, not boundary detection.
   If multilingual corpora matter later, this is where the work is.
4. Chunk `text` duplicates the source span it names (`start_char..end_char`),
   so memory is roughly 2x document size during chunking. Fine at current
   document sizes, noted for very large single documents.

None of these blocks the lift. All four travel to whatever backlog exists when
one does.

## Deviations from the spec as drafted

1. Success criterion 4 listed `Pool` among forbidden strings. Struck during
   execution: it matched the crate's own mean-pooling vocabulary
   (`mean_pool_and_normalize`, `pooled`), and `ArangoPool`, the type it aimed
   at, is already caught by `arango`. The spec was amended in place with a note.
2. Success criterion 5 (tests pass with the Postgres cluster stopped) is
   satisfied by construction rather than by demonstration: the crate has zero
   dependencies and no I/O of any kind, so there is nothing that could reach a
   database. Stopping the cluster requires sudo, which the review session did
   not hold. Anyone wanting the literal demonstration can run
   `sudo systemctl stop yeomna-postgres && cargo test` and watch it pass.

## Verification transcript

- `cargo build`: clean.
- `cargo test`: 20 passed, 0 failed.
- `cargo clippy --all-targets`: no warnings.
- `cargo fmt --check`: clean.
- `diff -r` against the reference: `strategies.rs` and `late.rs` identical,
  `lib.rs` differs from `mod.rs` by the five-line provenance note only.
- Store-reference grep (`arango`, `sql`, `postgres`): clean.
