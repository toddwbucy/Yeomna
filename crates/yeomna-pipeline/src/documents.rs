//! The document ingest operation (spec 015): walk, sort, hash-skip,
//! convert through the extraction seam, chunk, write, and draw the graph
//! the corpus declares in its own notation. A second pass links source
//! files to the claims they name.
//!
//! Two disciplines borrowed whole from the codebase orchestrator: a file
//! that will not convert is a counted refusal and the batch continues,
//! and an unchanged file is skipped for conversion and writing on the
//! strength of a stored content hash, so R8 and R9 hold for documents
//! exactly as they do for code. Skipped for writing, never for
//! declaration: every file's graph blocks are read and re-declared on
//! every run, which is what makes a run interrupted between a document
//! and its declarations, or an edge the sink rejected, repair itself on
//! the next run instead of hiding behind the hash.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use yeomna_chunking::ChunkingStrategy;
use yeomna_code::Language;
use yeomna_embed::extraction::ExtractOptions;
use yeomna_keys as keys;

use crate::document_graph::{GraphEdge, GraphNode, parse_graph_blocks, scan_conforms};
use crate::extract::Extractor;
use crate::orchestrator::{PipelineError, chunk_doc};
use crate::probe::IngestProbe;
use crate::profile;
use crate::sink::IngestSink;

/// The containers this operation writes. Documents and chunks are the
/// default profile's, edges are the shared edge container.
const DOCUMENTS: &str = "documents";
const CHUNKS: &str = "chunks";
const EMBEDDINGS: &str = "embeddings";
const EDGES: &str = "edges";

/// The analyzer names the edges carry, which is how a reader tells a
/// corpus-declared edge from an analyzer-derived one.
const GRAPH_BLOCK: &str = "graph-block";
const CONFORMS_HEADER: &str = "conforms-header";

/// How a document ingest runs.
#[derive(Debug, Clone)]
pub struct DocumentsConfig {
    /// Replace existing rows. Deterministic keys make this safe.
    pub overwrite: bool,
    /// Skip files larger than this.
    pub max_file_bytes: u64,
    /// The extensions the walk offers to the extractor, without dots.
    /// Spec 015 fixes this at Markdown.
    pub extensions: Vec<String>,
}

impl Default for DocumentsConfig {
    fn default() -> Self {
        Self {
            overwrite: true,
            max_file_bytes: 1024 * 1024,
            extensions: vec!["md".to_string()],
        }
    }
}

/// What a run did. Every field is a count the caller can report, and the
/// refusals name their files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DocumentsSummary {
    pub files_seen: usize,
    pub files_written: usize,
    /// Content hash matched the stored head.
    pub files_skipped: usize,
    /// Could not be read or converted, named in `refusals`.
    pub files_failed: usize,
    pub files_oversized: usize,
    /// A second file normalizing to a key already taken this run. The
    /// first in sorted order won (EC-4).
    pub collisions: usize,
    pub chunks_written: usize,
    pub blocks_seen: usize,
    pub blocks_refused: usize,
    /// Corpus-declared nodes written.
    pub nodes_declared: usize,
    /// Declarations that named an existing non-document node and left it
    /// alone: the doc graph landing on the code graph.
    pub nodes_fused: usize,
    /// Endpoints nothing had declared or ingested, created as
    /// placeholders awaiting promotion.
    pub placeholders: usize,
    pub edges_declared: usize,
    pub edges_rejected: usize,
    /// Repeated declarations of one node key or one edge identity across
    /// the corpus. The first in sorted order wins and the rest are
    /// counted here rather than written over it.
    pub duplicates: usize,
    /// Declared edges by relation, as the corpus named them.
    pub relations: BTreeMap<String, usize>,
    /// Every per-file and per-block refusal, as `path: reason`.
    pub refusals: Vec<String>,
}

/// One file the walk offered, held until the batch is sorted.
struct Offered {
    rel_path: String,
    path: std::path::PathBuf,
    len: u64,
}

/// A node or edge one document declared, remembered with where.
struct Declared {
    doc_key: String,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

/// Walk `root` for the configured extensions, sorted by relative path so
/// every downstream choice is the same on every run.
fn walk(root: &Path, config: &DocumentsConfig) -> Vec<Offered> {
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
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !config.extensions.contains(&ext) {
            continue;
        }
        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(Offered {
            rel_path,
            path: path.to_path_buf(),
            len,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

/// Ingest the documents under `root` into the sink's graph.
pub async fn ingest_documents<S, X>(
    root: &Path,
    sink: &S,
    extractor: &X,
    chunker: &(dyn ChunkingStrategy + Send + Sync),
    config: &DocumentsConfig,
) -> Result<DocumentsSummary, PipelineError>
where
    S: IngestSink + IngestProbe,
    X: Extractor,
{
    let mut summary = DocumentsSummary::default();
    let mut taken: HashSet<String> = HashSet::new();
    let mut declared: Vec<Declared> = Vec::new();

    for file in walk(root, config) {
        summary.files_seen += 1;
        if file.len > config.max_file_bytes {
            info!(rel_path = file.rel_path, "dropped, over the size limit");
            summary.files_oversized += 1;
            continue;
        }
        let doc_key = keys::normalize_document_key(&file.rel_path);
        if !taken.insert(doc_key.clone()) {
            summary.collisions += 1;
            summary.refusals.push(format!(
                "{}: normalizes to {doc_key:?}, already taken this run",
                file.rel_path
            ));
            continue;
        }
        let bytes = match std::fs::read(&file.path) {
            Ok(b) => b,
            Err(e) => {
                summary.files_failed += 1;
                summary.refusals.push(format!("{}: {e}", file.rel_path));
                continue;
            }
        };
        let content_hash: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // The blocks are read from the source, not the export, so a
        // converter that reflows fences cannot hide a declaration. And
        // they are read before the hash-skip, for every file: skipped for
        // conversion and writing, never for declaration.
        let blocks = std::str::from_utf8(&bytes)
            .map(parse_graph_blocks)
            .unwrap_or_default();
        let block_count = blocks.blocks_seen;
        summary.blocks_seen += block_count;
        summary.blocks_refused += blocks.refusals.len();
        for r in &blocks.refusals {
            summary
                .refusals
                .push(format!("{}:{}: {}", file.rel_path, r.line, r.reason));
        }
        if !blocks.nodes.is_empty() || !blocks.edges.is_empty() {
            declared.push(Declared {
                doc_key: doc_key.clone(),
                nodes: blocks.nodes,
                edges: blocks.edges,
            });
        }

        let stored = sink
            .stored_document_hash(&doc_key)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        if stored.as_deref() == Some(content_hash.as_str()) {
            summary.files_skipped += 1;
            continue;
        }

        let extracted = match extractor
            .extract_file(&file.path, ExtractOptions::all())
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // FR5: typed, counted, named, and the batch continues.
                warn!(rel_path = file.rel_path, error = %e, "not converted");
                summary.files_failed += 1;
                summary.refusals.push(format!("{}: {e}", file.rel_path));
                continue;
            }
        };

        let headings: Value = extracted
            .metadata
            .get("headings")
            .and_then(|h| serde_json::from_str(h).ok())
            .unwrap_or(Value::Array(Vec::new()));
        let chunks = chunker.chunk(&extracted.full_text);
        let doc = json!({
            "_key": doc_key,
            "path": file.rel_path,
            "title": extracted.metadata.get("title"),
            "headings": headings,
            "content_hash": content_hash,
            "extractor": extracted.metadata.get("extractor"),
            "full_text": extracted.full_text,
            "chunk_count": chunks.len(),
            "graph_blocks": block_count,
        });
        let out = sink
            .insert_documents(DOCUMENTS, &[doc], config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.files_written += out.created;

        // A shorter re-conversion would leave chunk rows at the higher
        // indices, so stale chunks and embeddings go first, exactly as
        // the document flow's five-call sequence does it.
        if config.overwrite {
            for container in [CHUNKS, EMBEDDINGS] {
                sink.remove_documents_by_fields(
                    container,
                    &["doc_key", profile::DEFAULT.foreign_key],
                    &doc_key,
                )
                .await
                .map_err(|e| PipelineError::Sink(Box::new(e)))?;
            }
        }
        if !chunks.is_empty() {
            let chunk_docs: Vec<Value> = chunks
                .iter()
                .enumerate()
                .map(|(i, c)| chunk_doc(&profile::DEFAULT, &doc_key, i, c))
                .collect();
            let out = sink
                .insert_documents(CHUNKS, &chunk_docs, config.overwrite)
                .await
                .map_err(|e| PipelineError::Sink(Box::new(e)))?;
            summary.chunks_written += out.created;
        }
    }

    write_declared(sink, config, &declared, &mut summary).await?;
    info!(?summary, "document ingest complete");
    Ok(summary)
}

/// The corpus's declarations become rows: nodes, then placeholders for
/// endpoints nothing declared, then edges. In that order so every edge
/// resolves.
async fn write_declared<S>(
    sink: &S,
    config: &DocumentsConfig,
    declared: &[Declared],
    summary: &mut DocumentsSummary,
) -> Result<(), PipelineError>
where
    S: IngestSink + IngestProbe,
{
    // Nodes. A declaration that names an existing code node fuses onto it
    // and leaves it alone: the corpus may describe its crates, and the
    // code ingest's row for one is the better row.
    let mut declared_keys: HashSet<String> = HashSet::new();
    let mut node_docs: Vec<Value> = Vec::new();
    for d in declared {
        for n in &d.nodes {
            // First declaration wins, the walk being sorted, and a second
            // one is a count rather than an overwrite.
            if !declared_keys.insert(n.key.clone()) {
                summary.duplicates += 1;
                continue;
            }
            let existing = sink
                .stored_kind(&n.key)
                .await
                .map_err(|e| PipelineError::Sink(Box::new(e)))?;
            if existing.as_deref().is_some_and(|k| k != "document") {
                summary.nodes_fused += 1;
                continue;
            }
            let mut doc = json!({
                "_key": n.key,
                "declared_kind": n.kind,
                "tag": n.tag,
                "declared_in": d.doc_key,
                "declared_at_line": n.line,
            });
            for (k, v) in &n.extra {
                doc[k] = json!(v);
            }
            node_docs.push(doc);
        }
    }
    if !node_docs.is_empty() {
        let out = sink
            .insert_documents(DOCUMENTS, &node_docs, config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.nodes_declared += out.created;
    }

    // Placeholders, never overwriting: an endpoint that already exists in
    // any form is left exactly as it is, and one that does not gets a row
    // to hang the edge on, promoted in place by whatever ingests it later.
    let mut placeholder_docs: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for d in declared {
        for e in &d.edges {
            for endpoint in [&e.from, &e.to] {
                if declared_keys.contains(endpoint) || !seen.insert(endpoint.clone()) {
                    continue;
                }
                let existing = sink
                    .stored_kind(endpoint)
                    .await
                    .map_err(|e| PipelineError::Sink(Box::new(e)))?;
                if existing.is_none() {
                    placeholder_docs.push(json!({
                        "_key": endpoint,
                        "placeholder": true,
                        "first_named_in": d.doc_key,
                    }));
                }
            }
        }
    }
    if !placeholder_docs.is_empty() {
        let out = sink
            .insert_documents(DOCUMENTS, &placeholder_docs, false)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.placeholders += out.created;
    }

    // Edges, relation names as the corpus wrote them, one row per R6
    // identity however many times the corpus restates it.
    let mut edge_docs: Vec<Value> = Vec::new();
    let mut identities: HashSet<(String, String, String)> = HashSet::new();
    for d in declared {
        for e in &d.edges {
            if !identities.insert((e.from.clone(), e.to.clone(), e.relation.clone())) {
                summary.duplicates += 1;
                continue;
            }
            *summary.relations.entry(e.relation.clone()).or_insert(0) += 1;
            let mut payload = json!({
                "declared_in": d.doc_key,
                "declared_at_line": e.line,
            });
            for (k, v) in &e.extra {
                payload[k] = json!(v);
            }
            edge_docs.push(json!({
                "from": e.from,
                "to": e.to,
                "relation": e.relation,
                "basis": "declared",
                "analyzer": GRAPH_BLOCK,
                "payload": payload,
            }));
        }
    }
    if !edge_docs.is_empty() {
        let out = sink
            .insert_documents(EDGES, &edge_docs, config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.edges_declared += out.created;
        summary.edges_rejected += out.errors;
    }
    Ok(())
}

/// What the conforms pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConformsSummary {
    pub files_scanned: usize,
    pub headers_seen: usize,
    /// Headers whose slug did not parse, named in `refusals`.
    pub malformed: usize,
    pub edges_written: usize,
    /// Edges the sink could not attach, which with the claim checked
    /// beforehand means the file node itself is missing: the code was
    /// not ingested into this graph.
    pub edges_rejected: usize,
    /// Slugs no node carries, each named once. A header pointing at a
    /// retired claim is a finding the graph owes its operator (EC-2).
    pub unresolved: Vec<String>,
    /// The same file naming the same claim more than once: one edge,
    /// the first line kept, the rest counted here.
    pub duplicates: usize,
    pub refusals: Vec<String>,
}

/// Link every `conforms:` header under `root` to the claim it names:
/// one declared edge from the file node to the claim node, analyzer
/// `conforms-header`. Runs after the code and the documents are in.
pub async fn link_conforms<S>(
    root: &Path,
    sink: &S,
    config: &DocumentsConfig,
) -> Result<ConformsSummary, PipelineError>
where
    S: IngestSink + IngestProbe,
{
    let mut summary = ConformsSummary::default();
    let mut known: HashMap<String, bool> = HashMap::new();
    let mut unresolved: HashSet<String> = HashSet::new();
    let mut linked: HashSet<(String, String)> = HashSet::new();
    let mut edge_docs: Vec<Value> = Vec::new();

    let mut files: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
    for entry in ignore::WalkBuilder::new(root).hidden(false).build() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        if Language::from_path(&rel_path).is_none() {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        files.push((rel_path, path.to_path_buf(), len));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, path, len) in files {
        if len > config.max_file_bytes {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        summary.files_scanned += 1;
        let scan = scan_conforms(&source);
        summary.headers_seen += scan.headers.len();
        summary.malformed += scan.malformed.len();
        for r in &scan.malformed {
            summary
                .refusals
                .push(format!("{rel_path}:{}: {}", r.line, r.reason));
        }
        if scan.headers.is_empty() {
            continue;
        }
        let file_key = keys::file_key(&rel_path);
        for h in scan.headers {
            if !linked.insert((file_key.clone(), h.slug.clone())) {
                summary.duplicates += 1;
                continue;
            }
            let exists = match known.get(&h.slug) {
                Some(v) => *v,
                None => {
                    let v = sink
                        .stored_kind(&h.slug)
                        .await
                        .map_err(|e| PipelineError::Sink(Box::new(e)))?
                        .is_some();
                    known.insert(h.slug.clone(), v);
                    v
                }
            };
            if !exists {
                unresolved.insert(h.slug);
                continue;
            }
            edge_docs.push(json!({
                "from": file_key,
                "to": h.slug,
                "relation": "conforms",
                "basis": "declared",
                "analyzer": CONFORMS_HEADER,
                "payload": {"path": rel_path, "line": h.line},
            }));
        }
    }

    if !edge_docs.is_empty() {
        let out = sink
            .insert_documents(EDGES, &edge_docs, config.overwrite)
            .await
            .map_err(|e| PipelineError::Sink(Box::new(e)))?;
        summary.edges_written += out.created;
        summary.edges_rejected += out.errors;
    }
    summary.unresolved = unresolved.into_iter().collect();
    summary.unresolved.sort();
    info!(?summary, "conforms pass complete");
    Ok(summary)
}
