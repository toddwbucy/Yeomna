# Review Notes: yeomna-batch lift

Reviewer: Claude, with Todd. Date: 2026-08-10. Every file was read in full at
spec time and the diff verified at move time.

## Accounting

| Source (`crates/hades-core/src/batch/`) | Destination | Lines | Diff class |
|---|---|---|---|
| `mod.rs` | `src/lib.rs` | 17 | Provenance note, then FR-B4/FR-B5 docs in tightening |
| `error.rs` | `src/error.rs` | 28 | Byte-identical at move |
| `processor.rs` | `src/processor.rs` | 583 | Byte-identical at move |
| `progress.rs` | `src/progress.rs` | 228 | Byte-identical at move |
| `rate_limit.rs` | `src/rate_limit.rs` | 143 | Byte-identical at move |
| `state.rs` | `src/state.rs` | 221 | Byte-identical at move |

1,220 lines, 27 tests moved and passing.

## Commit structure

1. **Move**: five files byte-identical, `lib.rs` differs by provenance only.
2. **Tightening**: FR-B4 default state filename to
   `.yeomna-batch-state.json` (one code line, wire format untouched,
   `test_python_compat` unmodified), and FR-B5 resume semantics documented in
   the crate docs.

## What the review found

**A third dependency correction: `anyhow`.** Used fully qualified in the
public `process()` signature with no `use` statement, so grep-based surveys
missed it, the same way they missed `thiserror` (attribute macro) and
`chrono` (inline path). The pattern is now established: dependency tables in
specs are estimates until the compiler confirms them, and the compiler is
the authority.

**`anyhow::Error` in a library's public API** is a reference design choice
the move preserves. The reference's own convention (its CLAUDE.md) is
thiserror for libraries and anyhow for binaries, and `process()` taking
`anyhow::Error` sits against that grain. Recorded for a future CodeRabbit
conversation rather than changed, since the signature is what callers in
Phase 5 will compile against.

**Timing-based tests noted per EC-B3.** `test_rate_limit_acquire` asserts at
least 80ms elapsed on a 100ms budget. Passing consistently here, left alone,
loosened only if CI ever flakes.

**stderr progress reporting** writes through `ProgressReporter`. Worth
remembering when the daemon consumes it later, since a daemon's stderr goes
to the journal. CodeRabbit review hardened the write so a closed stderr
cannot panic the batch.

**Per-item checkpointing, reviewed and kept.** CodeRabbit flagged
`record_result` rewriting the full state file per item as O(N squared).
Correct arithmetic, deliberate semantics: per-item durability is the crash
resilience the checkpoint exists for, and interval checkpointing would lose
up to an interval of completed work on crash. Revisit if a measured ingest
shows the write cost, not before.

## Verification

- 27 tests passing, clippy clean, fmt clean.
- `diff` against the reference: five byte-identical files.
- Store-reference grep (`arango`, `sql`, `postgres`): clean at every commit,
  since batch never referenced a store.
- `hades` grep after tightening: only the provenance citation remains.
