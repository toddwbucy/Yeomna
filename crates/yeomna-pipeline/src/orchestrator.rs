//! Pipeline orchestrator — the core extract → chunk → embed → store flow.

use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use tracing::{debug, error, info, instrument, warn};

use crate::profile::CollectionProfile;
use crate::sink::IngestSink;
use yeomna_chunking::{ChunkingStrategy, TextChunk};
use yeomna_embed::embedding::{EmbedResult, EmbeddingClient, EmbeddingError};
use yeomna_embed::extraction::{ExtractOptions, ExtractResult, ExtractionClient, ExtractionError};
use yeomna_keys as keys;

/// Pipeline configuration.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Collection profile to store results in.
    pub profile: &'static CollectionProfile,
    /// Embedding task parameter (e.g. "retrieval.passage").
    pub embed_task: String,
    /// Embedding batch size (None = server default).
    pub embed_batch_size: Option<u32>,
    /// Extraction options.
    pub extract_options: ExtractOptions,
    /// Whether to overwrite existing documents.
    pub overwrite: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            profile: &crate::profile::DEFAULT,
            embed_task: "retrieval.passage".to_string(),
            embed_batch_size: None,
            extract_options: ExtractOptions::all(),
            overwrite: true,
        }
    }
}

/// Error type for pipeline operations.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("extraction failed: {0}")]
    Extraction(#[from] ExtractionError),

    #[error("embedding failed: {0}")]
    Embedding(#[from] EmbeddingError),

    #[error("sink error: {0}")]
    Sink(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("chunking produced no content")]
    EmptyChunks,

    #[error("pipeline error: {0}")]
    Other(String),
}

/// Result of processing a single document.
#[derive(Debug)]
pub struct DocumentResult {
    /// Document key (normalized from source identifier).
    pub doc_key: String,
    /// Whether processing succeeded.
    pub success: bool,
    /// Number of chunks produced.
    pub chunk_count: usize,
    /// Wall-clock time for this document.
    pub duration_ms: u64,
    /// Error message if processing failed.
    pub error: Option<String>,
}

/// Summary of a batch pipeline run.
#[derive(Debug)]
pub struct PipelineSummary {
    /// Per-document results.
    pub results: Vec<DocumentResult>,
    /// Total documents processed.
    pub total: usize,
    /// Successful documents.
    pub succeeded: usize,
    /// Failed documents.
    pub failed: usize,
    /// Total wall-clock time in milliseconds.
    pub total_duration_ms: u64,
}

impl PipelineSummary {
    /// Build a summary from a list of document results.
    pub fn from_results(results: Vec<DocumentResult>, total_duration_ms: u64) -> Self {
        let total = results.len();
        let succeeded = results.iter().filter(|r| r.success).count();
        Self {
            total,
            succeeded,
            failed: total - succeeded,
            results,
            total_duration_ms,
        }
    }
}

/// The document processing pipeline.
///
/// Orchestrates extraction, chunking, embedding, and storage for
/// documents flowing into the Yeomna knowledge graph.
pub struct Pipeline<S: IngestSink> {
    extractor: ExtractionClient,
    embedder: EmbeddingClient,
    sink: S,
    config: PipelineConfig,
}

impl<S: IngestSink> Pipeline<S> {
    /// Create a new pipeline with the given service clients, sink, and config.
    pub fn new(
        extractor: ExtractionClient,
        embedder: EmbeddingClient,
        sink: S,
        config: PipelineConfig,
    ) -> Self {
        Self {
            extractor,
            embedder,
            sink,
            config,
        }
    }

    /// Process a single document through the full pipeline.
    ///
    /// Extracts content, chunks text, embeds chunks, and stores everything
    /// through the configured sink containers.
    #[instrument(skip(self, chunker), fields(doc_id))]
    pub async fn process_document(
        &self,
        file_path: &Path,
        doc_id: &str,
        chunker: &(dyn ChunkingStrategy + Send + Sync),
    ) -> DocumentResult {
        let start = Instant::now();
        let doc_key = keys::normalize_document_key(doc_id);

        match self.process_inner(file_path, &doc_key, chunker).await {
            Ok(chunk_count) => {
                let duration = start.elapsed().as_millis() as u64;
                info!(
                    doc_key,
                    chunk_count,
                    duration_ms = duration,
                    "document processed"
                );
                DocumentResult {
                    doc_key,
                    success: true,
                    chunk_count,
                    duration_ms: duration,
                    error: None,
                }
            }
            Err(e) => {
                let duration = start.elapsed().as_millis() as u64;
                error!(doc_key, error = %e, "document processing failed");
                DocumentResult {
                    doc_key,
                    success: false,
                    chunk_count: 0,
                    duration_ms: duration,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    /// Process a batch of documents with two-phase GPU optimization.
    ///
    /// **Phase 1** — Extract all documents (VLM stays loaded on GPU).
    /// **Phase 2** — Chunk and embed all extracted content (embedder loaded).
    ///
    /// Per-document errors are isolated: one failure does not abort the batch.
    #[instrument(skip(self, chunker, documents), fields(batch_size = documents.len()))]
    pub async fn process_batch(
        &self,
        documents: &[(&Path, &str)],
        chunker: &(dyn ChunkingStrategy + Send + Sync),
    ) -> PipelineSummary {
        let batch_start = Instant::now();
        let mut results = Vec::with_capacity(documents.len());

        // -- Phase 1: Extract all documents --------------------------------
        info!(count = documents.len(), "phase 1: extracting documents");
        // (doc_key, extract_result, extraction_duration_ms)
        let mut extractions: Vec<Option<(String, ExtractResult, u64)>> =
            Vec::with_capacity(documents.len());

        for &(path, doc_id) in documents {
            let doc_key = keys::normalize_document_key(doc_id);
            let extract_start = Instant::now();
            match self
                .extractor
                .extract_file(path, self.config.extract_options.clone())
                .await
            {
                Ok(result) => {
                    let extraction_ms = extract_start.elapsed().as_millis() as u64;
                    debug!(
                        doc_key,
                        text_len = result.full_text.len(),
                        extraction_ms,
                        "extracted"
                    );
                    extractions.push(Some((doc_key, result, extraction_ms)));
                }
                Err(e) => {
                    let extraction_ms = extract_start.elapsed().as_millis() as u64;
                    error!(doc_key, error = %e, "extraction failed");
                    results.push(DocumentResult {
                        doc_key,
                        success: false,
                        chunk_count: 0,
                        duration_ms: extraction_ms,
                        error: Some(format!("extraction: {e}")),
                    });
                    extractions.push(None);
                }
            }
        }

        // -- Phase 2: Chunk + Embed + Store --------------------------------
        info!("phase 2: chunking, embedding, and storing");
        for extraction in extractions {
            let Some((doc_key, extract_result, extraction_ms)) = extraction else {
                continue; // already recorded as failed
            };

            let phase2_start = Instant::now();
            match self
                .chunk_embed_store(&doc_key, &extract_result, chunker)
                .await
            {
                Ok(chunk_count) => {
                    let duration = extraction_ms + phase2_start.elapsed().as_millis() as u64;
                    info!(doc_key, chunk_count, duration_ms = duration, "stored");
                    results.push(DocumentResult {
                        doc_key,
                        success: true,
                        chunk_count,
                        duration_ms: duration,
                        error: None,
                    });
                }
                Err(e) => {
                    let duration = extraction_ms + phase2_start.elapsed().as_millis() as u64;
                    error!(doc_key, error = %e, "chunk/embed/store failed");
                    results.push(DocumentResult {
                        doc_key,
                        success: false,
                        chunk_count: 0,
                        duration_ms: duration,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        let total_duration = batch_start.elapsed().as_millis() as u64;
        let summary = PipelineSummary::from_results(results, total_duration);
        info!(
            total = summary.total,
            succeeded = summary.succeeded,
            failed = summary.failed,
            duration_ms = summary.total_duration_ms,
            "batch complete"
        );
        summary
    }

    // -----------------------------------------------------------------------
    // Internal pipeline steps
    // -----------------------------------------------------------------------

    /// Full single-document pipeline: extract → chunk → embed → store.
    async fn process_inner(
        &self,
        file_path: &Path,
        doc_key: &str,
        chunker: &(dyn ChunkingStrategy + Send + Sync),
    ) -> Result<usize, PipelineError> {
        // 1. Extract
        let extract_result = self
            .extractor
            .extract_file(file_path, self.config.extract_options.clone())
            .await?;

        // 2-4. Chunk, embed, store
        self.chunk_embed_store(doc_key, &extract_result, chunker)
            .await
    }

    /// Chunk extracted text, embed chunks, store through the sink.
    async fn chunk_embed_store(
        &self,
        doc_key: &str,
        extract_result: &ExtractResult,
        chunker: &(dyn ChunkingStrategy + Send + Sync),
    ) -> Result<usize, PipelineError> {
        // 2. Chunk
        let chunks = chunker.chunk(&extract_result.full_text);
        if chunks.is_empty() {
            warn!(doc_key, "chunking produced no text chunks");
            return Err(PipelineError::EmptyChunks);
        }

        // 3. Embed
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embed_result = self
            .embedder
            .embed(
                &texts,
                &self.config.embed_task,
                self.config.embed_batch_size,
            )
            .await?;

        if embed_result.embeddings.len() != chunks.len() {
            return Err(PipelineError::Other(format!(
                "embedding count mismatch: expected {} (chunks), got {} (embeddings)",
                chunks.len(),
                embed_result.embeddings.len()
            )));
        }

        // 4. Store
        self.store(doc_key, extract_result, &chunks, &embed_result)
            .await?;

        Ok(chunks.len())
    }

    /// Store metadata, chunks, and embeddings through the sink.
    async fn store(
        &self,
        doc_key: &str,
        extract_result: &ExtractResult,
        chunks: &[TextChunk],
        embed_result: &EmbedResult,
    ) -> Result<(), PipelineError> {
        let profile = self.config.profile;

        // -- Delete stale chunks/embeddings for this doc_key -----------------
        // When overwriting, a rerun with fewer chunks would leave orphans.
        if self.config.overwrite {
            self.delete_doc_chunks(profile, doc_key).await?;
        }

        // -- Metadata document ---------------------------------------------
        let metadata_doc = json!({
            "_key": doc_key,
            "full_text": extract_result.full_text,
            "tables": extract_result.tables.len(),
            "equations": extract_result.equations.len(),
            "images": extract_result.images.len(),
            "chunk_count": chunks.len(),
            "embedding_model": embed_result.model,
            "embedding_dimension": embed_result.dimension,
            "extractor_metadata": extract_result.metadata,
        });

        let meta_res = self
            .sink
            .insert_documents(profile.metadata, &[metadata_doc], self.config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        if meta_res.errors > 0 {
            return Err(PipelineError::Other(format!(
                "metadata import had {} errors",
                meta_res.errors
            )));
        }

        // -- Chunk documents -----------------------------------------------
        let chunk_docs: Vec<Value> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| chunk_doc(profile, doc_key, i, chunk))
            .collect();

        let chunk_res = self
            .sink
            .insert_documents(profile.chunks, &chunk_docs, self.config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        if chunk_res.errors > 0 {
            return Err(PipelineError::Other(format!(
                "chunk import had {} errors (created {})",
                chunk_res.errors, chunk_res.created
            )));
        }

        // -- Embedding documents -------------------------------------------
        let embedding_docs: Vec<Value> = embed_result
            .embeddings
            .iter()
            .enumerate()
            .map(|(i, emb)| embedding_doc(profile, doc_key, i, emb))
            .collect();

        let emb_res = self
            .sink
            .insert_documents(profile.embeddings, &embedding_docs, self.config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        if emb_res.errors > 0 {
            return Err(PipelineError::Other(format!(
                "embedding import had {} errors (created {})",
                emb_res.errors, emb_res.created
            )));
        }

        debug!(
            doc_key,
            chunks = chunks.len(),
            "stored metadata, chunks, and embeddings"
        );
        Ok(())
    }

    /// Delete existing chunk and embedding documents for a doc_key.
    ///
    /// Prevents stale rows when a rerun produces fewer chunks than before.
    async fn delete_doc_chunks(
        &self,
        profile: &CollectionProfile,
        doc_key: &str,
    ) -> Result<(), PipelineError> {
        // Deletes go through the sink's remove-by-fields operation. In the
        // reference this shape (one bind object per query) is what made the
        // store's declared-but-unused parameter defect (every `--force`
        // refresh aborting, #169) unrepresentable rather than merely fixed,
        // and a sink implementation should preserve that property.
        //
        // The filter matches `doc_key` (what this pipeline has always written)
        // OR the profile's declared foreign key: rows written by the legacy
        // Python pipeline carry only the foreign key (`parent_key`), so a
        // doc_key-only filter would leave them behind on refresh —
        // accumulating stale chunks exactly like #159 did on the parsed code
        // path (#165).
        //
        // The two removes are NOT transactional: chunks commit before
        // embeddings run, so a transient failure of the second call leaves
        // orphaned embeddings until the next successful overwrite of the same
        // document, which clears them (each run deletes before writing).
        // The window could be closed by folding both removes into one
        // multi-container operation or a transaction, which is the store
        // PRD's M3 decision to make, not this trait's. The reference chose
        // the same non-atomic shape deliberately (reuse over atomicity),
        // and the orphan state is accepted because it self-heals and is
        // read-invisible (search joins embeddings to chunks that no longer
        // exist and drops them).
        let match_fields = ["doc_key", profile.foreign_key];
        self.sink
            .remove_documents_by_fields(profile.chunks, &match_fields, doc_key)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        self.sink
            .remove_documents_by_fields(profile.embeddings, &match_fields, doc_key)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;

        debug!(doc_key, "deleted stale chunks and embeddings");
        Ok(())
    }
}

impl<S: IngestSink> std::fmt::Debug for Pipeline<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("profile", &self.config.profile)
            .field("embed_task", &self.config.embed_task)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Document construction
// ---------------------------------------------------------------------------

/// Build a chunk document carrying BOTH `doc_key` and the profile's declared
/// foreign key (#165).
///
/// The search path filters on `profile.foreign_key` (`parent_key` for the
/// default profile), while the delete/refresh path has historically filtered
/// on `doc_key`. Writing only `doc_key` made every natively-ingested document
/// invisible to `db query` — silently, since zero results is a valid answer.
/// Writing both, with the field name taken from the SAME `CollectionProfile`
/// the reader uses, makes writer/reader agreement structural rather than
/// conventional.
fn chunk_doc(profile: &CollectionProfile, doc_key: &str, index: usize, chunk: &TextChunk) -> Value {
    let mut doc = json!({
        "_key": keys::chunk_key(doc_key, index),
        "doc_key": doc_key,
        "text": chunk.text,
        "chunk_index": chunk.chunk_index,
        "total_chunks": chunk.total_chunks,
        "start_char": chunk.start_char,
        "end_char": chunk.end_char,
    });
    set_foreign_key(&mut doc, profile, doc_key);
    doc
}

/// Write the profile's foreign key into `doc`, guarding against a profile
/// whose `foreign_key` collides with a structural field.
///
/// Safe for both registered profiles today (`parent_key`, `file_key`), and
/// `foreign_key == "doc_key"` is a harmless same-value overwrite. But a future
/// profile naming a structural field (`text`, `embedding`, `chunk_key`, ...)
/// would silently clobber real data, so that case is a debug-time panic and a
/// release-time refusal-to-clobber. The
/// `profile_foreign_keys_do_not_collide_with_structural_fields` test pins this
/// for every registered profile.
fn set_foreign_key(doc: &mut Value, profile: &CollectionProfile, doc_key: &str) {
    let fk = profile.foreign_key;
    let existing = doc.get(fk);
    let collides = existing.is_some_and(|v| v.as_str() != Some(doc_key));
    debug_assert!(
        !collides,
        "profile foreign_key '{fk}' collides with a structural document field"
    );
    if !collides {
        doc[fk] = json!(doc_key);
    }
}

/// Build an embedding document. Same dual-key contract as [`chunk_doc`].
fn embedding_doc(
    profile: &CollectionProfile,
    doc_key: &str,
    index: usize,
    embedding: &[f32],
) -> Value {
    let ck = keys::chunk_key(doc_key, index);
    let mut doc = json!({
        "_key": keys::embedding_key(&ck),
        "chunk_key": ck,
        "doc_key": doc_key,
        "embedding": embedding,
    });
    set_foreign_key(&mut doc, profile, doc_key);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

    fn chunk() -> TextChunk {
        TextChunk {
            text: "hello".into(),
            start_char: 0,
            end_char: 5,
            chunk_index: 0,
            total_chunks: 1,
        }
    }

    /// The writer must emit the field the reader filters on, for every profile
    /// (#165). The reader (db_search phase 1) filters
    /// `emb.<profile.foreign_key> != null`, so a chunk/embedding row missing
    /// that field is invisible to search.
    #[test]
    fn docs_carry_the_profile_foreign_key() {
        for (name, profile) in [
            ("default", &profile::DEFAULT),
            ("codebase", &profile::CODEBASE),
        ] {
            let c = chunk_doc(profile, "docA", 0, &chunk());
            let e = embedding_doc(profile, "docA", 0, &[0.1, 0.2]);
            for (kind, d) in [("chunk", &c), ("embedding", &e)] {
                assert_eq!(
                    d[profile.foreign_key].as_str(),
                    Some("docA"),
                    "{kind} doc for profile '{name}' missing its declared foreign key '{}' — search cannot find it",
                    profile.foreign_key
                );
                assert_eq!(
                    d["doc_key"].as_str(),
                    Some("docA"),
                    "{kind} doc lost the legacy doc_key contract the delete/refresh path relies on"
                );
            }
        }
    }

    /// No registered profile may name a structural chunk/embedding field as
    /// its foreign key — that would make `set_foreign_key` clobber real data.
    #[test]
    fn profile_foreign_keys_do_not_collide_with_structural_fields() {
        let structural = [
            "_key",
            "text",
            "chunk_index",
            "total_chunks",
            "start_char",
            "end_char",
            "chunk_key",
            "embedding",
        ];
        for (name, p) in [
            ("default", &profile::DEFAULT),
            ("codebase", &profile::CODEBASE),
        ] {
            let fk = p.foreign_key;
            assert!(
                !structural.contains(&fk),
                "profile '{name}' foreign_key '{fk}' names a structural field"
            );
        }
    }

    /// Keys and linkage stay stable under the refactor.
    #[test]
    fn doc_construction_shape_unchanged() {
        let profile = &profile::DEFAULT;
        let c = chunk_doc(profile, "docA", 3, &chunk());
        assert_eq!(c["_key"], "docA_chunk_3");
        assert_eq!(c["text"], "hello");
        let e = embedding_doc(profile, "docA", 3, &[1.0]);
        assert_eq!(e["chunk_key"], "docA_chunk_3");
        assert_eq!(e["_key"], "docA_chunk_3_emb");
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use crate::sink::{IngestSink, InsertOutcome};
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock sink failure")]
    struct MockError;

    #[derive(Default)]
    struct MockSink {
        calls: Mutex<Vec<String>>,
    }

    impl IngestSink for MockSink {
        type Error = MockError;

        async fn insert_documents(
            &self,
            container: &str,
            documents: &[Value],
            overwrite: bool,
        ) -> Result<InsertOutcome, MockError> {
            self.calls.lock().unwrap().push(format!(
                "insert:{container}:{}:{overwrite}",
                documents.len()
            ));
            Ok(InsertOutcome {
                created: documents.len(),
                errors: 0,
            })
        }

        async fn remove_documents_by_fields(
            &self,
            container: &str,
            fields: &[&str],
            key: &str,
        ) -> Result<(), MockError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("remove:{container}:{}:{key}", fields.join("+")));
            Ok(())
        }
    }

    /// Compile-time proof: the pipeline is generic over any sink. Exercising
    /// the full five-call store sequence through `Pipeline` waits for the
    /// sink-implementation PR, because `ExtractionClient::connect` is eager
    /// and a `Pipeline` cannot be constructed without a live extractor.
    #[allow(dead_code)]
    fn pipeline_accepts_any_sink(p: Pipeline<MockSink>) -> Pipeline<MockSink> {
        p
    }

    #[tokio::test]
    async fn mock_sink_implements_the_contract() {
        let sink = MockSink::default();
        let out = sink
            .insert_documents("documents", &[json!({"_key": "a"})], true)
            .await
            .unwrap();
        assert_eq!(out.created, 1);
        assert_eq!(out.errors, 0);
        sink.remove_documents_by_fields("chunks", &["doc_key", "parent_key"], "a")
            .await
            .unwrap();
        let calls = sink.calls.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            [
                "insert:documents:1:true".to_string(),
                "remove:chunks:doc_key+parent_key:a".to_string(),
            ]
        );
    }
}
