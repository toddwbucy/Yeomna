//! Transitional container naming, carried so the Phase 5 lift stays a move.
//!
//! The reference's `CollectionProfile` registry (308 lines: `get(name)`,
//! environment lookup, profile listing) was refused in Phases 1 and 4 and is
//! refused here again. This module is the four-field struct and the two
//! consts the orchestrator's one caller needs, verbatim values, nothing
//! else. **The sink strips the meaning these names encode** once the store
//! schema exists, so this module must not grow a registry, a lookup, or a
//! builder (spec 005).

/// A set of three container names plus the foreign-key field linking
/// chunks and embeddings back to their parent document.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CollectionProfile {
    /// Document metadata container.
    pub metadata: &'static str,
    /// Text chunk container.
    pub chunks: &'static str,
    /// Embedding vector container.
    pub embeddings: &'static str,
    /// Field in chunk/embedding documents referencing the parent document
    /// key (`"parent_key"` for documents, `"file_key"` for codebase).
    pub foreign_key: &'static str,
}

/// The generic document profile, verbatim from the reference.
pub static DEFAULT: CollectionProfile = CollectionProfile {
    metadata: "documents",
    chunks: "chunks",
    embeddings: "embeddings",
    foreign_key: "parent_key",
};

/// The codebase profile, verbatim from the reference.
pub static CODEBASE: CollectionProfile = CollectionProfile {
    metadata: "codebase_files",
    chunks: "codebase_chunks",
    embeddings: "codebase_embeddings",
    foreign_key: "file_key",
};
