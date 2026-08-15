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
use tracing::{debug, info, warn};

use yeomna_chunking::ChunkingStrategy;
use yeomna_code::{
    AnalysisOptions, AnalyzerOutcome, FileAnalysis, Language, Symbol, analyze_with_fallback, lsp,
    python_calls, rust_imports, tree_sitter_edges,
};
use yeomna_embed::embedding::EmbeddingClient;
use yeomna_keys as keys;

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
    /// The embedding task name passed to the embedder.
    pub embed_task: String,
    /// Run rust-analyzer over the Rust crates for semantically resolved
    /// `calls` and `implements` edges, and for the semantic half of the
    /// enrichment protocol.
    ///
    /// Off by default, and deliberately so. It puts an external process in
    /// the ingest path, it waits for a workspace index, and it turns a
    /// two-second run into a minute-scale one. When it fails it degrades
    /// to the syn-only graph rather than failing the ingest, because a
    /// language server that will not start is an environment problem and
    /// the structural graph is still worth having.
    pub semantic_rust: bool,
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
            semantic_rust: false,
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
    /// Symbol nodes written.
    pub symbols_written: usize,
    /// Edges written, across every basis.
    pub edges_written: usize,
    /// Edges whose endpoints did not resolve to nodes. Counted, never
    /// silently dropped (spec 011 FR 6).
    pub edges_unresolved: usize,
    /// Chunks written.
    pub chunks_written: usize,
    /// Embeddings written.
    pub embeddings_written: usize,
    /// Crates the semantic pass indexed, zero when it did not run.
    pub semantic_crates: usize,
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
pub async fn ingest_codebase<S>(
    root: &Path,
    sink: &S,
    chunker: &(dyn ChunkingStrategy + Send + Sync),
    embedder: Option<&EmbeddingClient>,
    config: &CodebaseConfig,
) -> Result<CodebaseSummary, PipelineError>
where
    S: IngestSink + IngestProbe,
{
    if config.embed && embedder.is_none() {
        return Err(PipelineError::Other(
            "embedding requested with no embedder supplied".into(),
        ));
    }
    let mut summary = CodebaseSummary::default();
    let analyzed = walk_and_analyze(root, sink, config, &mut summary).await?;

    // Pass 1: nodes. Everything the edge pass will point at must exist.
    for file in analyzed.iter().filter(|f| f.changed) {
        write_file_nodes(sink, file, config, &mut summary).await?;
        write_chunks_and_embeddings(sink, file, chunker, embedder, config, &mut summary).await?;
    }

    // Pass 1b: the semantic enrichment, when asked for. Symbols written a
    // second time by a language server converge onto the same rows through
    // the same keys, which is the Phase 7 enrichment protocol, and the
    // endpoints the semantic edges need must exist before pass 2.
    let semantic = if config.semantic_rust {
        semantic_rust_pass(root, sink, &analyzed, config, &mut summary).await
    } else {
        Vec::new()
    };

    // Pass 2: edges, now that every endpoint is resolvable.
    write_edges(sink, &analyzed, semantic, config, &mut summary).await?;

    info!(
        seen = summary.files_seen,
        written = summary.files_written,
        skipped = summary.files_skipped,
        failed = summary.files_failed,
        symbols = summary.symbols_written,
        edges = summary.edges_written,
        unresolved = summary.edges_unresolved,
        "codebase ingest complete"
    );
    Ok(summary)
}

/// Walk the tree and analyze what it offers, honoring `.gitignore`.
async fn walk_and_analyze<S>(
    root: &Path,
    sink: &S,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<Vec<Analyzed>, PipelineError>
where
    S: IngestProbe,
{
    let mut out = Vec::new();
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
            debug!(rel_path, "skipped, over the size limit");
            continue;
        }
        // Not valid UTF-8 means not source, whatever the extension says.
        let Ok(source) = std::fs::read_to_string(path) else {
            summary.files_failed += 1;
            continue;
        };
        let analysis =
            match analyze_with_fallback(&source, lang, &rel_path, &AnalysisOptions::default()) {
                AnalyzerOutcome::Success(a) => a,
                AnalyzerOutcome::Failed { analyzer, reason } => {
                    // EC-1: one unparseable file does not fail the repository.
                    warn!(rel_path, analyzer, reason, "analysis failed");
                    summary.files_failed += 1;
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
    Ok(out)
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

/// Chunks, their symbol linkage, and optionally their embeddings.
async fn write_chunks_and_embeddings<S: IngestSink>(
    sink: &S,
    file: &Analyzed,
    chunker: &(dyn ChunkingStrategy + Send + Sync),
    embedder: Option<&EmbeddingClient>,
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Result<(), PipelineError> {
    let chunks = chunker.chunk(&file.source);
    if chunks.is_empty() {
        return Ok(());
    }
    let docs: Vec<Value> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // FR 4: the symbols this chunk covers, by key. The sink
            // resolves them to ids, since only it knows them.
            let symbol_keys: Vec<String> = file
                .analysis
                .symbols
                .iter()
                .filter(|s| covers(c.start_char, c.end_char, s, &file.source))
                .filter(|s| s.kind.universal_kind().is_some())
                .map(|s| keys::symbol_key(&file.file_key, &s.qualified_name(), s.start_line))
                .collect();
            json!({
                "_key": keys::chunk_key(&file.file_key, i),
                "doc_key": file.file_key,
                "file_key": file.file_key,
                "text": c.text,
                "chunk_index": c.chunk_index,
                "total_chunks": c.total_chunks,
                "start_char": c.start_char,
                "end_char": c.end_char,
                "symbol_keys": symbol_keys,
            })
        })
        .collect();
    let out = sink
        .insert_documents(CHUNKS, &docs, config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.chunks_written += out.created;

    if !config.embed {
        return Ok(());
    }
    let embedder = embedder.expect("checked at entry");
    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let result = embedder.embed(&texts, &config.embed_task, None).await?;
    if result.embeddings.len() != chunks.len() {
        return Err(PipelineError::Other(format!(
            "embedding count mismatch: expected {}, got {}",
            chunks.len(),
            result.embeddings.len()
        )));
    }
    let docs: Vec<Value> = result
        .embeddings
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let ck = keys::chunk_key(&file.file_key, i);
            json!({
                "_key": keys::embedding_key(&ck),
                "chunk_key": ck,
                "doc_key": file.file_key,
                "embedding": e,
            })
        })
        .collect();
    let out = sink
        .insert_documents(EMBEDDINGS, &docs, config.overwrite)
        .await
        .map_err(|e| PipelineError::Sink(Box::new(e)))?;
    summary.embeddings_written += out.created;
    Ok(())
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
    let by_lang = |want: Language| -> HashMap<String, Vec<Symbol>> {
        analyzed
            .iter()
            .filter(|f| f.analysis.language == want)
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

    // Everything else falls back to name matching over tree-sitter's
    // shape, which is the one place that resolver belongs.
    let others: HashMap<String, Vec<Symbol>> = analyzed
        .iter()
        .filter(|f| !matches!(f.analysis.language, Language::Rust | Language::Python))
        .map(|f| (f.rel_path.clone(), f.analysis.symbols.clone()))
        .collect();
    if !others.is_empty() {
        let structural = tree_sitter_edges::resolve(&others);
        for (relation, edges) in [
            ("calls", &structural.calls),
            ("imports", &structural.imports),
        ] {
            for e in edges {
                push_resolved(&mut docs, e, relation, "structural", &analyzer_of);
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
    summary.edges_unresolved += out.errors;
    Ok(())
}

/// The semantic Rust pass: rust-analyzer over each crate, for the symbols
/// and edges only a compiler front end can resolve.
///
/// Returns edge documents for the caller to write with the rest, and
/// writes the enriched symbol nodes itself, because the edges it returns
/// point at them.
///
/// Never fails the ingest. A language server that will not start, will not
/// index in time, or dies mid-crate leaves the structural graph standing
/// and says what happened.
async fn semantic_rust_pass<S: IngestSink>(
    root: &Path,
    sink: &S,
    analyzed: &[Analyzed],
    config: &CodebaseConfig,
    summary: &mut CodebaseSummary,
) -> Vec<Value> {
    let rust_files: Vec<std::path::PathBuf> = analyzed
        .iter()
        .filter(|f| f.analysis.language == Language::Rust)
        .map(|f| root.join(&f.rel_path))
        .collect();
    if rust_files.is_empty() {
        return Vec::new();
    }

    // rust-analyzer indexes a crate, not a file, so the files are grouped
    // by the manifest that owns them.
    let by_crate = lsp::group_files_by_crate(&rust_files);
    info!(crates = by_crate.len(), "starting the semantic Rust pass");

    let mut extraction: HashMap<String, lsp::symbols::FileExtraction> = HashMap::new();
    for (crate_root, files) in &by_crate {
        let session = match lsp::RustAnalyzerSession::start(crate_root).await {
            Ok(s) => s,
            Err(e) => {
                warn!(crate_root = %crate_root.display(), %e,
                      "no language server, keeping the structural graph");
                continue;
            }
        };
        if !session
            .wait_for_workspace_ready(config.semantic_timeout)
            .await
        {
            warn!(crate_root = %crate_root.display(),
                  "language server did not finish indexing in time, skipping this crate");
            continue;
        }
        let refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
        let extractor = lsp::RustSymbolExtractor::new(&session, true).with_path_root(root);
        for (path, data) in extractor.extract_crate(&refs).await {
            // Key on the path relative to the ingest root, so the keys
            // match the ones the structural pass already wrote.
            let rel = std::path::Path::new(&path)
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(path);
            extraction.insert(rel, data);
        }
    }
    if extraction.is_empty() {
        warn!("the semantic pass produced nothing, keeping the structural graph");
        return Vec::new();
    }

    let resolver = lsp::LspEdgeResolver::new(extraction, "rust-analyzer");

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
                info!(
                    enriched = out.created,
                    rejected = out.errors,
                    "semantic symbols enriched"
                );
            }
            Err(e) => {
                warn!(%e, "could not write enriched symbols, keeping the structural graph");
                return Vec::new();
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
            "analyzer": "rust-analyzer",
            "payload": e.metadata,
        }));
    }
    info!(edges = docs.len(), "semantic Rust edges resolved");
    summary.semantic_crates = by_crate.len();
    docs
}
