//! The pipeline's write boundary.
//!
//! Designed from its one caller, per `docs/specs/005-pipeline/spec.md`: the
//! orchestrator touches storage at five call sites through exactly two
//! operations, and this trait is that surface and nothing more. The store
//! crate implements it (store PRD Phase 7). Nothing else in this workspace
//! may, and the trait must not grow methods the caller does not call: no
//! transactions (the store PRD's M3 decision owns that), no queries, no
//! schema operations (the verb layer owns reads).
//!
//! The `overwrite` flag is load-bearing. It is the idempotent-reingest
//! switch, and the deterministic keys from `yeomna-keys` are what make
//! upsert-on-rerun safe.

/// Per-batch outcome of an insert.
#[derive(Debug, Clone, Copy)]
pub struct InsertOutcome {
    /// Documents created or replaced.
    pub created: usize,
    /// Documents the sink rejected.
    pub errors: usize,
}

/// The pipeline's storage sink.
///
/// Implementors provide durable, idempotent document storage addressed by
/// container name. Containers are named by the transitional
/// [`profile`](crate::profile) plumbing until the store schema exists.
pub trait IngestSink: Send + Sync {
    /// The sink's error type, surfaced through
    /// [`PipelineError::Sink`](crate::PipelineError::Sink).
    type Error: std::error::Error + Send + Sync + 'static;

    /// Batch-upsert JSON documents into a named container.
    ///
    /// With `overwrite` set, a document whose key already exists is
    /// replaced. Partial failure is reported through
    /// [`InsertOutcome::errors`] rather than an `Err`, matching the
    /// reference's import semantics that the orchestrator checks per batch.
    fn insert_documents(
        &self,
        container: &str,
        documents: &[serde_json::Value],
        overwrite: bool,
    ) -> impl std::future::Future<Output = Result<InsertOutcome, Self::Error>> + Send;

    /// Remove documents where any of `fields` equals `key`.
    ///
    /// Used to clear stale chunks and embeddings before an overwrite, so a
    /// rerun producing fewer chunks leaves no orphans.
    fn remove_documents_by_fields(
        &self,
        container: &str,
        fields: &[&str],
        key: &str,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}
