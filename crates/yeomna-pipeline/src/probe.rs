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

    /// Have any of these files' symbols been enriched by a language
    /// server.
    ///
    /// The semantic pass is expensive, so a unit whose files are all
    /// unchanged is skipped. Asking this per unit rather than per graph
    /// is what lets a crate whose server failed last run be retried
    /// without anyone editing its source, and a graph-wide answer would
    /// have called that crate done because some other crate succeeded.
    fn enrichment_present(
        &self,
        file_keys: &[String],
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send;

    /// The `content_hash` recorded for a document node, the document
    /// ingest's hash-skip (spec 015), or `None` when it was never
    /// ingested into this graph.
    fn stored_document_hash(
        &self,
        natural_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>, Self::Error>> + Send;

    /// The kind of the node under this key, or `None` when there is
    /// none. The document ingest asks before writing a corpus-declared
    /// node, so a declaration that names an existing code node fuses
    /// onto it rather than overwriting it, and before creating a
    /// placeholder, so an endpoint that exists in any form is left alone.
    fn stored_kind(
        &self,
        natural_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>, Self::Error>> + Send;
}
