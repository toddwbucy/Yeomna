//! Generic batch processing with concurrency, checkpointing, and progress.
//!
//! Provides [`BatchProcessor`] for running N items through an async pipeline
//! with per-item error isolation, resumable checkpoint state, throttled
//! progress reporting, and optional rate limiting.
//!
//! Lifted verbatim from HADES-Burn `crates/hades-core/src/batch/` per
//! `docs/specs/002-keys-and-batch/spec.md`. The only edits in the move are
//! this provenance note and `mod.rs` becoming `lib.rs`. Behavior is preserved
//! by the change being a move (PRD-pipeline-libraries G3), with one deviation
//! taken in tightening (spec FR-B4): the default state filename is
//! `.yeomna-batch-state.json`.
//!
//! Resume semantics worth knowing before use (spec FR-B5): a resumed batch
//! skips items that previously **failed**, not only items that completed.
//! [`BatchState::skip_set`] includes both. Retrying failures means passing
//! `reset` to clear the state file, and this is deliberate: a poisoned item
//! that crashes the pipeline would otherwise be retried on every resume,
//! defeating the fault isolation the checkpoint exists to provide.
//!
//! A skipped-because-failed item stays an **unresolved failure**: it counts
//! in the summary's `failed`, carries its stored error at stage `"resume"`,
//! and keeps the checkpoint file alive, so the semantics above hold across
//! any number of resumes rather than evaporating after the first one.

mod error;
mod processor;
mod progress;
mod rate_limit;
mod state;

pub use error::{BatchError, ItemError};
pub use processor::{BatchProcessor, BatchProcessorConfig, BatchSummary, ItemResult};
pub use progress::{ProgressEvent, ProgressReporter, ProgressStatus};
pub use rate_limit::RateLimiter;
pub use state::BatchState;
