//! Transitional container names, carried so the Phase 4 lift stays a move.
//!
//! These eight strings came from the reference's `db/collections.rs`
//! (`CODEBASE` static, values verbatim). They name the reference store's
//! containers, and the analysis layer folds them into endpoint strings like
//! `codebase_files/src_lib_rs`. **The sink strips the prefix they encode**,
//! so these names have no future in the schema. This module must not grow a
//! registry, a profile, or a builder (spec 004 FR-C2). The reference's
//! 308-line `CollectionProfile` machinery stays behind on purpose.

/// Extended collection set for codebase ingestion.
///
/// Beyond the standard metadata/chunks/embeddings triple, codebase
/// analysis produces symbol-level metadata and typed graph edges.
/// Each edge type has its own collection (collection-per-relation).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodebaseCollections {
    /// File-level metadata (language, metrics, symbol_hash).
    pub files: &'static str,
    /// AST-aligned text chunks.
    pub chunks: &'static str,
    /// Embedding vectors per chunk.
    pub embeddings: &'static str,
    /// Symbol-level metadata (name, kind, span, parent file).
    pub symbols: &'static str,
    /// Edge collection: file defines symbol.
    pub defines_edges: &'static str,
    /// Edge collection: symbol calls symbol.
    pub calls_edges: &'static str,
    /// Edge collection: symbol implements trait method.
    pub implements_edges: &'static str,
    /// Edge collection: file imports symbol or file.
    pub imports_edges: &'static str,
}

pub static CODEBASE: CodebaseCollections = CodebaseCollections {
    files: "codebase_files",
    chunks: "codebase_chunks",
    embeddings: "codebase_embeddings",
    symbols: "codebase_symbols",
    defines_edges: "codebase_defines_edges",
    calls_edges: "codebase_calls_edges",
    implements_edges: "codebase_implements_edges",
    imports_edges: "codebase_imports_edges",
};
