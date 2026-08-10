//! Document chunking engine.
//!
//! Pure Rust text chunking with multiple strategies.  Each strategy
//! implements the [`ChunkingStrategy`] trait, producing [`TextChunk`]
//! values with character offsets for source attribution.
//!
//! Late-chunking support (mean-pooling token-level embeddings per chunk)
//! is in the [`late`] submodule.
//!
//! Lifted verbatim from HADES-Burn `crates/hades-core/src/chunking/` per
//! `docs/specs/001-workspace-and-chunking/spec.md`. The only edits are this
//! provenance note and `mod.rs` becoming `lib.rs`. Behavior is preserved by
//! the change being a move (PRD-pipeline-libraries G3).

mod late;
mod strategies;

pub use late::{LateChunkConfig, LateChunkResult, late_chunk_embeddings};
pub use strategies::{SentenceChunking, SlidingWindowChunking, TokenChunking};

/// A chunk of text with positional metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    /// The chunk text content.
    pub text: String,
    /// Start byte offset in the original document (inclusive).
    pub start_char: usize,
    /// End byte offset in the original document (exclusive).
    pub end_char: usize,
    /// Zero-based chunk index within the document.
    pub chunk_index: usize,
    /// Total number of chunks produced from the document.
    ///
    /// Populated by the chunking strategy before returning.
    pub total_chunks: usize,
}

/// Strategy for splitting text into chunks.
pub trait ChunkingStrategy {
    /// Split `text` into a vector of [`TextChunk`]s.
    ///
    /// The returned chunks have `total_chunks` already filled in.
    fn chunk(&self, text: &str) -> Vec<TextChunk>;
}
