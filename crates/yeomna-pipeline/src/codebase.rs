//! The codebase ingest orchestrator, per `docs/specs/011-ingest-orchestrator`.
//!
//! Walk a tree, analyze each file, skip what has not changed, chunk, embed,
//! and write nodes and edges through the sink. New construction against
//! lifted parts: it replaces `codebase_ingest.rs`, which was excluded from
//! the port as store-coupled, and no line of it was consulted.
//!
//! **Two passes, and the reason is D1.** Cross-file edges name symbols in
//! files the walk has not reached yet, so every node is written before any
//! edge is. That makes endpoint resolution total rather than best-effort,
//! at the cost of holding the symbol map until the walk finishes.
//!
//! **Unchanged files still contribute their symbols.** A skipped file is
//! skipped for writing, not for resolution: an edge pointing into it must
//! still resolve, so its symbols enter the map either way.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};
use tracing::{info, warn};

use yeomna_chunking::ChunkingStrategy;
use yeomna_code::{
    AnalysisOptions, AnalyzerOutcome, FileAnalysis, Language, Symbol, analyze_with_fallback,
    cpp_edges, lsp, python_calls, rust_imports, tree_sitter_edges,
};
use yeomna_embed::embedding::{ChunkPolicy, EmbeddedChunk, EmbeddingError};
use yeomna_keys as keys;

use crate::embed::Embedder;
use crate::orchestrator::PipelineError;
use crate::probe::IngestProbe;
use crate::sink::IngestSink;

/// Containers this orchestrator writes. The document flow uses
/// `profile::CODEBASE` for three of them, and symbols and edges have no
/// profile entry because the document flow never writes them.
const FILES: &str = "codebase_files";
const SYMBOLS: &str = "codebase_symbols";
const CHUNKS: &str = "codebase_chunks";
const EMBEDDINGS: &str = "codebase_embeddings";
const EDGES: &str = "codebase_edges";

/// How a codebase ingest runs.
#[derive(Debug, Clone)]
pub struct CodebaseConfig {
    /// Replace existing rows. Deterministic keys are what make this safe.
    pub overwrite: bool,
    /// Embed chunks. Off by default: no embedder ships yet (H4), and a
    /// graph without vectors is still a graph. When on, an unreachable
    /// embedder fails the run rather than leaving a half-ingest that looks
    /// complete (spec 011 EC-4).
    pub embed: bool,
    /// Skip files larger than this. Generated and vendored blobs are not
    /// worth an analyzer pass.
    pub max_file_bytes: u64,
    /// The embedding task name passed to the embedder. It reaches the
    /// `embeddings.task` column, so it is the corpus half of the pairing a
    /// query has to match (R26).
    pub embed_task: String,
    /// Where the late-chunk boundaries fall, in tokens. Sent to the
    /// service as request fields, so the chunking policy stays here and
    /// the service executes it rather than owning it.
    pub chunking: ChunkPolicy,
    /// Run the language servers (rust-analyzer over Rust crates, gopls
    /// over Go modules) for semantically resolved `calls` and
    /// `implements` edges, and for the semantic half of the enrichment
    /// protocol.
    ///
    /// C++ needs no flag: libclang runs in process during analysis and
    /// records its call sites in symbol metadata, so `cpp_edges` resolves
    /// them for free on the structural path.
    ///
    /// Off by default, and deliberately so. It puts an external process in
    /// the ingest path, it waits for a workspace index, and it turns a
    /// two-second run into a minute-scale one. When it fails it degrades
    /// to the syn-only graph rather than failing the ingest, because a
    /// language server that will not start is an environment problem and
    /// the structural graph is still worth having.
    pub semantic_lsp: bool,
    /// How long to wait for the language server to finish indexing before
    /// giving up on the semantic pass.
    pub semantic_timeout: std::time::Duration,
}

impl Default for CodebaseConfig {
    fn default() -> Self {
        Self {
            overwrite: true,
            embed: false,
            max_file_bytes: 1024 * 1024,
            embed_task: "retrieval.passage".to_string(),
            chunking: ChunkPolicy::default(),
            semantic_lsp: false,
            semantic_timeout: std::time::Duration::from_secs(180),
        }
    }
}

/// What a run did. Every field is a count the caller can report.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodebaseSummary {
    /// Files the walk offered.
    pub files_seen: usize,
    /// Files analyzed and written.
    pub files_written: usize,
    /// Files whose `symbol_hash` matched the stored head.
    pub files_skipped: usize,
    /// Files that could not be read or analyzed.
    pub files_failed: usize,
    /// Files dropped for exceeding `max_file_bytes`, never analyzed.
    /// Separate from `files_skipped`, which means the hash matched.
    pub files_oversized: usize,
    /// Symbol nodes written.
    pub symbols_written: usize,
    /// Edges written, across every basis.
    pub edges_written: usize,
    /// Edges the sink rejected: an endpoint that resolved to no node, a
    /// malformed document, or, under `overwrite: false`, an edge that was
    /// already stored. Unresolved endpoints are the interesting case and
    /// FR 6 is why nothing is dropped silently, but the count is wider
    /// than that one cause and the name should not overpromise.
    pub edges_rejected: usize,
    /// Chunks written.
    pub chunks_written: usize,
    /// Embeddings written.
    pub embeddings_written: usize,
    /// Files whose text was above the embedder's context ceiling. They are
    /// chunked and searchable by keyword and carry no vectors (PRD D5).
    /// Counted apart from everything else because "present and too long to
    /// encode" is not "absent", which is the distinction spec 019's drift
    /// finding was about.
    pub files_over_ceiling: usize,
    /// Crates and modules the semantic pass indexed, zero when it did
    /// not run or found no server.
    pub semantic_units: usize,
    /// Symbol rows the semantic pass wrote. Counted apart from
    /// `symbols_written` because most of these land on rows the
    /// structural pass already created, which is the enrichment protocol
    /// rather than new symbols.
    pub symbols_enriched: usize,
}

/// A file the walk offered and could not assess: over the size limit,
/// unreadable, or unparseable. Its key matters to drift, because the
/// graph may hold a node for it and that node is not stale, only
/// unverifiable this run.
struct Skipped {
    rel_path: String,
    file_key: String,
    reason: &'static str,
}

/// One analyzed file, held until the node pass completes.
struct Analyzed {
    rel_path: String,
    file_key: String,
    analysis: FileAnalysis,
    source: String,
    /// False when the stored hash matched, meaning this file contributes
    /// symbols for resolution but no writes.
    changed: bool,
}

/// Ingest a source tree into the graph the sink is scoped to.
///
/// `embedder` is generic rather than concrete so a test can pass
/// `HashEmbedder` and exercise this whole path with no GPU. A caller that
/// wants no embedding passes `None::<&EmbeddingClient>`, naming a type the
/// inference cannot guess from `None` alone.
pub async fn ingest_codebase<S, E>(
    root: &Path,
    sink: &S,
    chunker: &(dyn ChunkingStrategy + Send + Sync),
    embedder: Option<&E>,
    config: &CodebaseConfig,
) -> Result<CodebaseSummary, PipelineError>
where
    S: IngestSink + IngestProbe,
    E: Embedder,
{
    if config.embed && embedder.is_none() {
        return Err(PipelineError::Other(
            "embedding requested with no embedder supplied".into(),
        ));
    }
    let mut summary = CodebaseSummary::default();
    let (analyzed, _skipped) = walk_and_analyze(root, sink, config, &mut summary).await?;

    // Pass 1: nodes. Everything the edge pass will point at must exist.
    for file in analyzed.iter().filter(|f| f.changed) {
        write_file_nodes(sink, file, config, &mut summary).await?;
        write_chunks_and_embeddings(sink, file, chunker, embedder, config, &mut summary).await?;
    }

    // Pass 1b: the semantic enrichment, when asked for. Symbols written a
    // second time by a language server converge onto the same rows through
    // the same keys, which is the Phase 7 enrichment protocol, and the
    // endpoints the semantic edges need must exist before pass 2.
    let semantic = if config.semantic_lsp {
        semantic_lsp_pass(root, sink, &analyzed, config, &mut summary).await?
    } else {
        Vec::new()
    };

    // Pass 2: edges, now that every endpoint is resolvable.
    write_edges(sink, root, &analyzed, semantic, config, &mut summary).await?;

    info!(
        seen = summary.files_seen,
        written = summary.files_written,
        skipped = summary.files_skipped,
        failed = summary.files_failed,
        symbols = summary.symbols_written,
        edges = summary.edges_written,
        rejected = summary.edges_rejected,
        "codebase ingest complete"
    );
    Ok(summary)
}

/// What a drift comparison found (D2). Paths rather than counts alone,
/// because an operator asked to trust a graph wants to see which files
/// the graph is wrong about.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DriftSummary {
    /// Files the walk offered, whether or not they could be assessed.
    pub files_seen: usize,
    /// Files whose stored `symbol_hash` matches. The graph is right
    /// about these.
    pub unchanged: usize,
    /// Files whose symbols hash differently than the graph holds.
    pub changed: Vec<String>,
    /// Files the tree has and the graph has never seen.
    pub new: Vec<String>,
    /// Files the graph holds and the tree no longer has. This is the
    /// list `retire` acts on, which is why a file the walk saw and could
    /// not assess never lands here: it is present, only unverifiable.
    pub missing: Vec<String>,
    /// Files the walk offered and could not assess, each with why. The
    /// graph may be right or wrong about these and this run cannot say.
    pub unassessed: Vec<String>,
}

impl DriftSummary {
    /// True when the graph matches the tree and nothing went unassessed.
    ///
    /// An unassessed file makes this false rather than being ignored. A
    /// caller asks `clean` to decide whether to trust an answer from the
    /// graph, and "matches, except for the files I could not read" is not
    /// a yes.
    pub fn clean(&self) -> bool {
        self.changed.is_empty()
            && self.new.is_empty()
            && self.missing.is_empty()
            && self.unassessed.is_empty()
    }
}

/// Compare a tree against what the graph holds, and write nothing (D2).
///
/// Shares the ingest's own walk and analysis, so the comparison is
/// against what an ingest would write rather than against a second idea
/// of it. That is the whole value: a drift that agreed with a different
/// analyzer than the one that fills the graph would report drift where
/// there is none, or miss it where there is.
pub async fn drift<S>(
    root: &Path,
    sink: &S,
    config: &CodebaseConfig,
) -> Result<DriftSummary, PipelineError>
where
    S: IngestProbe,
{
    let mut walked = CodebaseSummary::default();
    let (analyzed, skipped) = walk_and_analyze(root, sink, config, &mut walked).await?;
    let mut summary = DriftSummary {
        files_seen: analyzed.len() + skipped.len(),
        ..Default::default()
    };

    // Every key the walk saw, assessed or not. A file it could not read
    // is still a file the tree has, and calling it missing would tell
    // `retire` to sweep a node whose source is right there.
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for file in &analyzed {
        seen_keys.insert(file.file_key.clone());
        let stored = sink
            .stored_symbol_hash(&file.file_key)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        match stored {
            None => summary.new.push(file.rel_path.clone()),
            Some(h) if h == file.analysis.symbol_hash => summary.unchanged += 1,
            Some(_) => summary.changed.push(file.rel_path.clone()),
        }
    }
    for s in &skipped {
        seen_keys.insert(s.file_key.clone());
        summary
            .unassessed
            .push(format!("{}: {}", s.rel_path, s.reason));
    }

    for (key, path) in sink
        .stored_file_keys()
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?
    {
        if !seen_keys.contains(&key) {
            summary.missing.push(path);
        }
    }

    summary.changed.sort();
    summary.new.sort();
    summary.missing.sort();
    summary.unassessed.sort();
    info!(?summary, "drift complete");
    Ok(summary)
}

/// Walk the tree and analyze what it offers, honoring `.gitignore`.
async fn walk_and_analyze<S>(
    root: &Path,
    sink: &S,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<(Vec<Analyzed>, Vec<Skipped>), PipelineError>
where
    S: IngestProbe,
{
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for entry in ignore::WalkBuilder::new(root).hidden(false).build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(%e, "walk error");
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let Some(lang) = Language::from_path(&rel_path) else {
            continue;
        };
        summary.files_seen += 1;
        if entry.metadata().map(|m| m.len()).unwrap_or(0) > config.max_file_bytes {
            // Counted, so files_seen balances against the outcome fields.
            // Not files_skipped, which means the hash matched.
            info!(rel_path, "dropped, over the size limit");
            summary.files_oversized += 1;
            skipped.push(Skipped {
                file_key: keys::file_key(&rel_path),
                rel_path,
                reason: "over the size limit",
            });
            continue;
        }
        // Not valid UTF-8 means not source, whatever the extension says.
        let Ok(source) = std::fs::read_to_string(path) else {
            summary.files_failed += 1;
            skipped.push(Skipped {
                file_key: keys::file_key(&rel_path),
                rel_path,
                reason: "could not be read",
            });
            continue;
        };
        let analysis =
            match analyze_with_fallback(&source, lang, &rel_path, &AnalysisOptions::default()) {
                AnalyzerOutcome::Success(a) => a,
                AnalyzerOutcome::Failed { analyzer, reason } => {
                    // EC-1: one unparseable file does not fail the repository.
                    warn!(rel_path, analyzer, reason, "analysis failed");
                    summary.files_failed += 1;
                    skipped.push(Skipped {
                        file_key: keys::file_key(&rel_path),
                        rel_path,
                        reason: "could not be analyzed",
                    });
                    continue;
                }
            };
        let file_key = keys::file_key(&rel_path);
        let stored = sink
            .stored_symbol_hash(&file_key)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        let changed = stored.as_deref() != Some(analysis.symbol_hash.as_str());
        if !changed {
            summary.files_skipped += 1;
        }
        out.push(Analyzed {
            rel_path,
            file_key,
            analysis,
            source,
            changed,
        });
    }
    Ok((out, skipped))
}

/// The file node and its symbol nodes.
async fn write_file_nodes<S: IngestSink>(
    sink: &S,
    file: &Analyzed,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<(), PipelineError> {
    let a = &file.analysis;
    let file_doc = json!({
        "_key": file.file_key,
        "path": file.rel_path,
        "language": format!("{:?}", a.language),
        "symbol_hash": a.symbol_hash,
        "analyzer": a.analyzer,
        "analysis_tier": format!("{:?}", a.analysis_tier),
        "fallback_reason": a.fallback_reason,
        "metrics": serde_json::to_value(&a.metrics).unwrap_or(Value::Null),
        "symbol_count": a.symbols.len(),
    });
    let out = sink
        .insert_documents(FILES, &[file_doc], config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.files_written += out.created;

    let symbol_docs: Vec<Value> = a
        .symbols
        .iter()
        .filter_map(|s| symbol_doc(&file.file_key, s))
        .collect();
    if !symbol_docs.is_empty() {
        let out = sink
            .insert_documents(SYMBOLS, &symbol_docs, config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.symbols_written += out.created;
    }
    Ok(())
}

/// A symbol node, or `None` for symbols that are not graph primitives
/// (imports and impl blocks, which `universal_kind` declines).
fn symbol_doc(file_key: &str, s: &Symbol) -> Option<Value> {
    let kind = s.kind.universal_kind()?;
    Some(json!({
        "_key": keys::symbol_key(file_key, &s.qualified_name(), s.start_line),
        "kind": kind,
        "name": s.name,
        "qualified_name": s.qualified_name(),
        "file_key": file_key,
        "start_line": s.start_line,
        "end_line": s.end_line,
        "metadata": s.metadata,
    }))
}

/// One piece of a file: its text, its byte span, and its vector when one
/// was produced.
///
/// Both chunking paths land here, which is what lets the write below be
/// written once. Under late chunking the boundaries come from the
/// embedder, because late chunking encodes the document first and decides
/// afterwards where the pieces were. Without embedding they come from the
/// local `ChunkingStrategy`, unchanged.
struct Piece {
    text: String,
    start_char: usize,
    end_char: usize,
    vector: Option<Vec<f32>>,
}

/// Turn a document's late chunks into pieces, checking the one thing the
/// contract promises and this code depends on: that a chunk's byte span
/// slices the text it came from.
fn pieces_from_late_chunks(
    source: &str,
    chunks: Vec<EmbeddedChunk>,
    rel_path: &str,
) -> Result<Vec<Piece>, String> {
    let (chunks, vectors) = crate::embed::late_pieces(source, chunks, rel_path)?;
    Ok(chunks
        .into_iter()
        .zip(vectors)
        .map(|(c, vector)| Piece {
            text: c.text,
            start_char: c.start_char,
            end_char: c.end_char,
            vector: Some(vector),
        })
        .collect())
}

/// Chunks, their symbol linkage, and optionally their embeddings.
async fn write_chunks_and_embeddings<S, E>(
    sink: &S,
    file: &Analyzed,
    chunker: &(dyn ChunkingStrategy + Send + Sync),
    embedder: Option<&E>,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<(), PipelineError>
where
    S: IngestSink,
    E: Embedder,
{
    // A file that shrinks would leave chunk rows at the higher indices,
    // and their embeddings with them, describing text the file no longer
    // contains. Upsert cannot remove them, so they go first, exactly as
    // the document flow's five-call sequence does it.
    if config.overwrite {
        for container in [CHUNKS, EMBEDDINGS] {
            sink.remove_documents_by_fields(container, &["doc_key", "file_key"], &file.file_key)
                .await
                .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        }
    }
    // EC-4: embed first when embeddings were asked for, so an unreachable
    // embedder fails before any chunk row exists. Writing chunks first
    // would leave text in the store with no vectors beside it, which is
    // the half-ingest EC-4 exists to prevent.
    //
    // Late chunking is why this decides the boundaries too rather than
    // only the vectors: the document is encoded in one pass and the chunk
    // vectors are conditioned on all of it, so the pieces are whatever the
    // pass says they were.
    // A file with nothing in it has nothing to embed, and asking the
    // service to embed it would be asking for a refusal. The local
    // chunker already returns no chunks for blank text, so the embedding
    // path agrees with it rather than turning an empty file into a failed
    // run. Found by ingesting a tree with an empty `.rs` file in it, which
    // real repositories have.
    let embeddable = embedder.filter(|_| config.embed && !file.source.trim().is_empty());
    let (pieces, identity) = match embeddable {
        Some(e) => match e
            .embed_document(&file.source, &config.embed_task, config.chunking)
            .await
        {
            Ok(chunks) => (
                pieces_from_late_chunks(&file.source, chunks, &file.rel_path)
                    .map_err(PipelineError::Other)?,
                Some(e.identity()),
            ),
            // PRD D5: a document above the ceiling is refused by the
            // service, never truncated. It is still worth having by
            // keyword, so it is chunked locally, counted, and named. The
            // coverage gap shows up through `read::health`'s
            // `chunks_without_embeddings`.
            Err(EmbeddingError::Service { ref code, .. }) if code == "input-too-large" => {
                warn!(
                    path = %file.rel_path,
                    "over the embedder's context ceiling, keyword only"
                );
                summary.files_over_ceiling += 1;
                (local_pieces(chunker, &file.source), None)
            }
            Err(e) => return Err(PipelineError::Embedding(e)),
        },
        None => (local_pieces(chunker, &file.source), None),
    };
    if pieces.is_empty() {
        return Ok(());
    }

    let total = pieces.len();
    let docs: Vec<Value> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| {
            // FR 4: the symbols this chunk covers, by key. The sink
            // resolves them to ids, since only it knows them.
            let symbol_keys: Vec<String> = file
                .analysis
                .symbols
                .iter()
                .filter(|s| covers(p.start_char, p.end_char, s, &file.source))
                .filter(|s| s.kind.universal_kind().is_some())
                .map(|s| keys::symbol_key(&file.file_key, &s.qualified_name(), s.start_line))
                .collect();
            json!({
                "_key": keys::chunk_key(&file.file_key, i),
                "doc_key": file.file_key,
                "file_key": file.file_key,
                "text": p.text,
                "chunk_index": i,
                "total_chunks": total,
                "start_char": p.start_char,
                "end_char": p.end_char,
                "symbol_keys": symbol_keys,
            })
        })
        .collect();

    let out = sink
        .insert_documents(CHUNKS, &docs, config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.chunks_written += out.created;
    // A rejected chunk means this code and the sink disagree about the
    // document shape, which is a defect here rather than something in the
    // corpus. Unlike an edge with an unresolved endpoint it is not a
    // legitimate partial, so it stops the run instead of riding a counter.
    // `errors` counts every row the sink did not create, and under
    // `overwrite: false` the insert is `ON CONFLICT DO NOTHING`, so a row
    // that was already there is indistinguishable from one that was
    // refused. Only under `overwrite` does the insert upsert, where a
    // non-creation can only mean a rejection. Checking it unconditionally
    // made a second ingest of a changed file abort the whole run, which is
    // worse than the discard it replaced.
    if config.overwrite && out.errors > 0 {
        return Err(PipelineError::Other(format!(
            "{}: the store rejected {} of {} chunk rows",
            file.rel_path, out.errors, total
        )));
    }

    let Some(identity) = identity else {
        return Ok(());
    };
    let docs: Vec<Value> = pieces
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let vector = p.vector.as_ref()?;
            let ck = keys::chunk_key(&file.file_key, i);
            Some(json!({
                "_key": keys::embedding_key(&ck),
                "chunk_key": ck,
                "doc_key": file.file_key,
                "embedding": vector,
                // R26: the cohort rides the row. It used to be read from
                // the parent node's payload, which this path never wrote.
                "model": identity.model,
                "model_revision": identity.model_revision,
                "task": config.embed_task,
            }))
        })
        .collect();
    let out = sink
        .insert_documents(EMBEDDINGS, &docs, config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.embeddings_written += out.created;
    if config.overwrite && out.errors > 0 {
        return Err(PipelineError::Other(format!(
            "{}: the store rejected {} of {} embedding rows",
            file.rel_path,
            out.errors,
            docs.len()
        )));
    }
    Ok(())
}

/// Chunk with the local strategy, for the paths that produce no vectors.
fn local_pieces(chunker: &(dyn ChunkingStrategy + Send + Sync), source: &str) -> Vec<Piece> {
    chunker
        .chunk(source)
        .into_iter()
        .map(|c| Piece {
            text: c.text,
            start_char: c.start_char,
            end_char: c.end_char,
            vector: None,
        })
        .collect()
}

/// Does a chunk's byte range cover a symbol's lines.
///
/// Chunk offsets are bytes and symbol positions are lines, so the line
/// numbers are converted rather than compared across units.
fn covers(start: usize, end: usize, s: &Symbol, source: &str) -> bool {
    let line_of = |offset: usize| source[..offset.min(source.len())].lines().count().max(1);
    let (first, last) = (line_of(start), line_of(end));
    s.start_line <= last && s.end_line >= first
}

/// Translate one resolver's edge document into the sink's edge shape.
///
/// Every resolver emits `_from` and `_to` as collection-qualified ids and
/// keeps its own detail alongside, so the detail rides into `payload`
/// whole and the sink strips the prefixes when it resolves endpoints.
fn push_resolved(
    docs: &mut Vec<Value>,
    e: &Value,
    relation: &str,
    basis: &str,
    analyzer_of: &HashMap<String, String>,
) {
    let (Some(from), Some(to)) = (
        e.get("_from").and_then(Value::as_str),
        e.get("_to").and_then(Value::as_str),
    ) else {
        return;
    };
    // Attribute to whatever ran on the source file, falling back to what
    // the resolver claimed. Never to nothing: the schema forbids it and Q1
    // is the reason.
    let source_key = from.rsplit('/').next().unwrap_or(from);
    let analyzer = analyzer_of
        .get(source_key)
        .map(String::as_str)
        .or_else(|| e.get("analyzer").and_then(Value::as_str))
        .unwrap_or("unattributed");
    docs.push(json!({
        "from": from,
        "to": to,
        "relation": relation,
        "basis": basis,
        "analyzer": analyzer,
        "payload": e,
    }));
}

/// Every edge, in one pass, after every node exists.
async fn write_edges<S: IngestSink>(
    sink: &S,
    root: &Path,
    analyzed: &[Analyzed],
    semantic: Vec<Value>,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<(), PipelineError> {
    // Nothing changed anywhere, so re-resolution cannot discover an edge
    // that is not already stored. Any change at all reopens the whole
    // corpus, because a new symbol in one file can be the target of an
    // import in a file that did not change.
    if !analyzed.iter().any(|f| f.changed) && semantic.is_empty() {
        return Ok(());
    }
    let mut docs: Vec<Value> = semantic;

    // `defines`, straight off the analysis. The file literally declares the
    // symbol, which is the parent PRD's Phase 4 definition of `declared`.
    for file in analyzed.iter().filter(|f| f.changed) {
        for s in &file.analysis.symbols {
            let Some(_) = s.kind.universal_kind() else {
                continue;
            };
            docs.push(json!({
                "from": file.file_key,
                "to": keys::symbol_key(&file.file_key, &s.qualified_name(), s.start_line),
                "relation": "defines",
                "basis": "declared",
                "analyzer": file.analysis.analyzer,
                "payload": {"analysis_tier": format!("{:?}", file.analysis.analysis_tier)},
            }));
        }
    }

    // Cross-file edges, each language through the best resolver it has.
    // tree-sitter is the fallback for languages with nothing better, not
    // the default: running it over Rust when syn produced the symbols
    // throws away fidelity that was already paid for, and it reads a
    // `calls` metadata field that syn does not write at all.
    // Cohorts are selected by the analyzer that actually ran, not by the
    // language. A Rust file that fell back to tree-sitter carries
    // tree-sitter's metadata shape, so syn's resolver would find nothing
    // in it and the fallback resolver would never see it.
    let fell_back = |f: &Analyzed| f.analysis.analyzer.contains("tree-sitter");
    let by_lang = |want: Language| -> HashMap<String, Vec<Symbol>> {
        analyzed
            .iter()
            .filter(|f| f.analysis.language == want && !fell_back(f))
            .map(|f| (f.rel_path.clone(), f.analysis.symbols.clone()))
            .collect()
    };
    // Which analyzer actually produced a file, so an edge is attributed to
    // the tool that ran rather than to a category.
    let analyzer_of: HashMap<String, String> = analyzed
        .iter()
        .map(|f| (f.file_key.clone(), f.analysis.analyzer.clone()))
        .collect();

    // Rust: syn extracted the use statements, so the import is readable
    // off the page, which is Phase 4's definition of `declared`.
    let rust = by_lang(Language::Rust);
    if !rust.is_empty() {
        let use_paths: HashMap<String, Vec<String>> = rust
            .iter()
            .map(|(p, syms)| (p.clone(), rust_imports::collect_use_paths(syms)))
            .collect();
        let index = rust_imports::build_symbol_index(&rust);
        for e in rust_imports::resolve_rust_imports(&use_paths, &index) {
            push_resolved(&mut docs, &e, "imports", "declared", &analyzer_of);
        }
    }

    // Python: the AST analyzer records call sites in symbol metadata and
    // this resolver reads them. An analyzer resolved the target rather
    // than the page stating it, so `structural`.
    let python = by_lang(Language::Python);
    if !python.is_empty() {
        let qualified = python_calls::build_qualified_index(&python);
        // The bare-name index the resolver's third stage wants. Only
        // `build_qualified_index` ships, so the caller builds this one.
        let mut bare: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (path, syms) in &python {
            let fkey = keys::file_key(path);
            for s in syms.iter().filter(|s| s.kind.universal_kind().is_some()) {
                bare.entry(s.name.clone()).or_default().push((
                    path.clone(),
                    keys::symbol_key(&fkey, &s.qualified_name(), s.start_line),
                ));
            }
        }
        for e in python_calls::resolve_python_calls(&python, &qualified, &bare) {
            push_resolved(&mut docs, &e, "calls", "structural", &analyzer_of);
        }
    }

    // C++: libclang runs in process during analysis and records call
    // sites with USRs and semantic resolution, so this needs no server and
    // no flag. A compiler front end resolved the target, which Phase 4
    // calls `structural`.
    let cpp = by_lang(Language::Cpp);
    if !cpp.is_empty() {
        for e in cpp_edges::resolve_cpp_calls(root, &cpp) {
            push_resolved(&mut docs, &e, "calls", "structural", &analyzer_of);
        }
    }

    // Everything tree-sitter produced, whatever its language, plus any
    // language with no resolver of its own. This is the one place that
    // resolver belongs, and it is reached by analyzer rather than by
    // language so a fallback parse is never orphaned.
    let others: HashMap<String, Vec<Symbol>> = analyzed
        .iter()
        .filter(|f| {
            fell_back(f)
                || !matches!(
                    f.analysis.language,
                    Language::Rust | Language::Python | Language::Cpp
                )
        })
        .map(|f| (f.rel_path.clone(), f.analysis.symbols.clone()))
        .collect();
    if !others.is_empty() {
        let structural = tree_sitter_edges::resolve(&others);
        // A call is resolved by matching names, which only an analyzer
        // could do. An import is written in the file, the same as the Rust
        // ones above, so Phase 4 makes it `declared`.
        for (relation, basis, edges) in [
            ("calls", "structural", &structural.calls),
            ("imports", "declared", &structural.imports),
        ] {
            for e in edges {
                push_resolved(&mut docs, e, relation, basis, &analyzer_of);
            }
        }
    }

    if docs.is_empty() {
        return Ok(());
    }
    let out = sink
        .insert_documents(EDGES, &docs, config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.edges_written += out.created;
    summary.edges_rejected += out.errors;
    Ok(())
}

/// The semantic pass: a language server per crate or module, for the
/// symbols and edges only a compiler front end can resolve.
///
/// Returns edge documents for the caller to write with the rest, and
/// writes the enriched symbol nodes itself, because the edges it returns
/// point at them.
///
/// Never fails the ingest. A language server that will not start, will not
/// index in time, or dies mid-crate leaves the structural graph standing
/// and says what happened.
async fn semantic_lsp_pass<S: IngestSink + IngestProbe>(
    root: &Path,
    sink: &S,
    analyzed: &[Analyzed],
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<Vec<Value>, PipelineError> {
    let files_for = |want: Language| -> Vec<std::path::PathBuf> {
        analyzed
            .iter()
            .filter(|f| f.analysis.language == want)
            .map(|f| root.join(&f.rel_path))
            .collect()
    };
    let rust_files = files_for(Language::Rust);
    let go_files = files_for(Language::Go);
    if rust_files.is_empty() && go_files.is_empty() {
        return Ok(Vec::new());
    }

    // Is this unit worth the cost of a language server. A unit with a
    // changed file always is. A unit with no changed file is worth it
    // only if it has never been enriched, which is both the first run
    // with the flag on and the retry after a server failed here before.
    let worth_indexing = |files: &[std::path::PathBuf]| {
        let keys: Vec<String> = files
            .iter()
            .filter_map(|abs| abs.strip_prefix(root).ok())
            .map(|rel| keys::file_key(&rel.to_string_lossy()))
            .collect();
        let changed = analyzed
            .iter()
            .any(|f| f.changed && keys.contains(&f.file_key));
        async move {
            if changed {
                return Ok::<bool, PipelineError>(true);
            }
            let enriched = sink
                .enrichment_present(&keys)
                .await
                .map_err(|e| PipelineError::Sink(Box::new(e)))?;
            Ok(!enriched)
        }
    };

    // Both servers index a unit larger than a file, so files are grouped
    // by the manifest that owns them: a crate for Rust, a module for Go.
    let mut extraction: HashMap<String, lsp::symbols::FileExtraction> = HashMap::new();
    let mut units = 0usize;

    // A document must land under the path relative to the ingest root, so
    // its key matches what the structural pass already wrote.
    let relativize = |path: String| -> String {
        match std::path::Path::new(&path).strip_prefix(root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => {
                // The key then matches nothing the structural pass wrote,
                // and the enrichment silently lands nowhere. Say so.
                warn!(
                    path,
                    root = %root.display(),
                    "extractor returned a path outside the ingest root, so its symbol keys will not match the structural pass"
                );
                path
            }
        }
    };

    if !rust_files.is_empty() {
        let by_crate = lsp::group_files_by_crate(&rust_files);
        info!(crates = by_crate.len(), "rust-analyzer pass");
        for (crate_root, files) in &by_crate {
            if !worth_indexing(files).await? {
                info!(unit = %crate_root.display(),
                      "unchanged and already enriched, skipping this crate");
                continue;
            }
            let session = match lsp::RustAnalyzerSession::start(crate_root).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(unit = %crate_root.display(), %e,
                          "no rust-analyzer, keeping the structural graph");
                    continue;
                }
            };
            if !session
                .wait_for_workspace_ready(config.semantic_timeout)
                .await
            {
                warn!(unit = %crate_root.display(),
                      "rust-analyzer did not index in time, skipping this crate");
                continue;
            }
            let refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
            let extractor = lsp::RustSymbolExtractor::new(&session, true).with_path_root(root);
            units += 1;
            for (path, data) in extractor.extract_crate(&refs).await {
                extraction.insert(relativize(path), data);
            }
        }
    }

    if !go_files.is_empty() {
        let by_module = lsp::group_files_by_go_module(&go_files);
        info!(modules = by_module.len(), "gopls pass");
        for (module_root, files) in &by_module {
            if !worth_indexing(files).await? {
                info!(unit = %module_root.display(),
                      "unchanged and already enriched, skipping this module");
                continue;
            }
            let session = match lsp::GoplsSession::start(module_root).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(unit = %module_root.display(), %e,
                          "no gopls, keeping the structural graph");
                    continue;
                }
            };
            if !session
                .wait_for_workspace_ready(config.semantic_timeout)
                .await
            {
                warn!(unit = %module_root.display(),
                      "gopls did not index in time, skipping this module");
                continue;
            }
            let refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
            let extractor =
                lsp::go_symbols::GoSymbolExtractor::new(&session, true).with_path_root(root);
            units += 1;
            for (path, data) in extractor.extract_module(&refs).await {
                extraction.insert(relativize(path), data);
            }
        }
    }

    if extraction.is_empty() {
        info!("the semantic pass had nothing to do, keeping the structural graph");
        return Ok(Vec::new());
    }

    // One resolver over both servers' output. The analyzer name is the
    // resolver's default label, and per-edge attribution below is what
    // actually reaches the store.
    let resolver = lsp::LspEdgeResolver::new(extraction, "language-server");

    // The semantic half of the enrichment protocol (Phase 7): the same
    // keys, richer payloads, landing on the rows syn already wrote.
    let symbol_docs: Vec<Value> = resolver
        .build_symbol_documents()
        .iter()
        .filter_map(|d| {
            let mut v = serde_json::to_value(d).ok()?;
            // The sink reads `kind` off the document for symbol nodes, and
            // SymbolDocument already carries the universal primitive there.
            v.as_object_mut()?.insert("enriched".into(), json!(true));
            Some(v)
        })
        .collect();
    if !symbol_docs.is_empty() {
        match sink
            .insert_documents(SYMBOLS, &symbol_docs, config.overwrite)
            .await
        {
            Ok(out) => {
                summary.symbols_enriched += out.created;
                info!(
                    enriched = out.created,
                    rejected = out.errors,
                    "semantic symbols enriched"
                );
            }
            Err(e) => {
                warn!(%e, "could not write enriched symbols, keeping the structural graph");
                return Ok(Vec::new());
            }
        }
    }

    // `defines` from a language server is still the file declaring the
    // symbol, so Phase 4 keeps it `declared`. Calls and implements are
    // what only an analyzer could resolve, which is `structural`.
    let mut docs = Vec::new();
    for e in resolver.build_edges() {
        let relation = e.kind.as_str();
        let basis = match e.kind {
            lsp::EdgeKind::Defines | lsp::EdgeKind::Imports => "declared",
            lsp::EdgeKind::Calls | lsp::EdgeKind::Implements => "structural",
        };
        docs.push(json!({
            "from": e.from,
            "to": e.to,
            "relation": relation,
            "basis": basis,
            "analyzer": analyzer_for(&e.from, analyzed),
            "payload": e.metadata,
        }));
    }
    info!(edges = docs.len(), units, "semantic edges resolved");
    summary.semantic_units = units;
    Ok(docs)
}

/// Which server produced an edge, from the language of its source file.
///
/// The endpoint is a collection-qualified id whose tail is either a file
/// key or a symbol key, and a symbol key is `{file_key}__{name}__{hash}`.
/// So the collection prefix comes off and the remainder is matched against
/// known file keys by prefix, longest first, or `src_a_rs` would claim
/// `src_a_rs_backup__...`. A miss is labelled for what it is rather than
/// guessed at, since the schema requires an analyzer and Q1 is the reason.
fn analyzer_for(from: &str, analyzed: &[Analyzed]) -> &'static str {
    let tail = from.split_once('/').map(|(_, rest)| rest).unwrap_or(from);
    let best = analyzed
        .iter()
        .filter(|f| tail == f.file_key || tail.starts_with(&format!("{}__", f.file_key)))
        .max_by_key(|f| f.file_key.len());
    match best.map(|f| f.analysis.language) {
        Some(Language::Go) => "gopls",
        Some(Language::Rust) => "rust-analyzer",
        Some(_) => "language-server",
        None => "language-server",
    }
}
