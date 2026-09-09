//! Document processing pipeline — extract → chunk → embed → store.
//!
//! Orchestrates the extraction and embedding clients with the chunking
//! engine and a storage sink. Supports single-document and batch processing
//! with two-phase GPU memory optimization.
//!
//! Lifted from HADES-Burn `crates/hades-core/src/pipeline/` per
//! `docs/specs/005-pipeline/spec.md`. This is the lift that is deliberately
//! part construction: the orchestrator's four store coupling points became
//! the [`IngestSink`] trait, defined in [`sink`] and implemented nowhere in
//! this workspace. The store crate implements it (store PRD Phase 7), and
//! until then the missing store is a typed hole rather than a design
//! discussion.

pub mod codebase;
pub mod document_graph;
pub mod documents;
pub mod extract;
mod orchestrator;
pub mod probe;
pub mod profile;
pub mod sink;

pub use codebase::{CodebaseConfig, CodebaseSummary, ingest_codebase};
pub use documents::{
    ConformsSummary, DocumentsConfig, DocumentsSummary, ingest_documents, link_conforms,
};
pub use extract::{ExtractError, Extractor, NativeExtractor};
pub use orchestrator::{DocumentResult, Pipeline, PipelineConfig, PipelineError, PipelineSummary};
pub use probe::IngestProbe;
pub use sink::{IngestSink, InsertOutcome};
