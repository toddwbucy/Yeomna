//! Generic batch processing with concurrency, checkpointing, and progress.
//!
//! Provides [`BatchProcessor`] for running N items through an async pipeline
//! with per-item error isolation, resumable checkpoint state, throttled
//! progress reporting, and optional rate limiting.
//!
//! Lifted verbatim from HADES-Burn `crates/hades-core/src/batch/` per
//! `docs/specs/002-keys-and-batch/spec.md`. The only edits in the move are
//! this provenance note and `mod.rs` becoming `lib.rs`. Behavior is preserved
//! by the change being a move (PRD-pipeline-libraries G3).

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
