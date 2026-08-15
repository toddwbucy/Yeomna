//! The one read the codebase orchestrator needs and the write boundary does
//! not provide.
//!
//! `IngestSink` is deliberately two methods and must not grow (spec 005),
//! but hash-skip is a read: the orchestrator has to know what a file's
//! symbols hashed to last time before it can decide to skip the work. That
//! read is this trait, kept separate so the write contract stays closed.
//!
//! The store implements both. Nothing else implements either.

/// A read over stored ingest state, used to skip unchanged work.
pub trait IngestProbe: Send + Sync {
    /// The probe's error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The `symbol_hash` recorded for a file node, or `None` when the file
    /// has never been ingested into this graph.
    fn stored_symbol_hash(
        &self,
        natural_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>, Self::Error>> + Send;
}
