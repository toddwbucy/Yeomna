//! The ingestion verbs (spec 019, PRD Phase 6, first half).
//!
//! H3 built the orchestrators and spec 015 built the document one, and
//! until now neither was reachable except from an ignored test. These
//! four verbs make ingestion something the appliance does rather than
//! something its test suite does.
//!
//! **Each opens its own connection.** The session holds its client
//! inside the lock that serializes calls (R16) and the orchestrators
//! take a `PgSink`, which owns a `Client`. Rather than reshape the
//! session around one verb, an ingesting verb connects for the duration
//! of the call and drops it at the end, exactly as `sql` does under
//! R17. The session's audit row still brackets the whole operation.
//!
//! **They block (D1).** A closed vocabulary has no job verbs, the
//! appliance is single-operator, and the audit row's NULL outcome is
//! already the in-progress signal. A caller waits.

use std::path::Path;

use serde_json::{Value, json};
use yeomna_chunking::TokenChunking;
use yeomna_pipeline::{
    CodebaseConfig, DocumentsConfig, NativeExtractor, drift as run_drift, ingest_codebase,
    ingest_documents, link_conforms,
};
use yeomna_store::PgSink;

use crate::error::VerbError;
use crate::execute::Exec;
use crate::verb::{DriftRequest, GraphScoped, IngestRequest};

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

/// The tree a caller named, checked before anything is opened (FR3).
fn readable_tree(path: &str) -> Result<&Path, VerbError> {
    let p = Path::new(path);
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => Ok(p),
        Ok(_) => Err(VerbError::InvalidArgs(format!(
            "{path:?} is not a directory"
        ))),
        Err(e) => Err(VerbError::InvalidArgs(format!("{path:?}: {e}"))),
    }
}

/// A sink on its own connection, bound to a graph that must already
/// exist. Creating one as a side effect of ingesting into it would make
/// a typo a new graph (FR1).
async fn sink_for(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    graph: &str,
) -> Result<PgSink, VerbError> {
    let exists = s
        .client()
        .query_opt("SELECT 1 FROM graphs WHERE name = $1", &[&graph])
        .await
        .map_err(db)?;
    if exists.is_none() {
        return Err(VerbError::NotFound(format!(
            "no graph named {graph:?}, create it before ingesting into it"
        )));
    }
    let Some((dir, port)) = endpoint else {
        return Err(VerbError::Internal(
            "this session was built without an endpoint, so an ingest cannot open its connection"
                .into(),
        ));
    };
    let database: String = s
        .client()
        .query_one("SELECT current_database()", &[])
        .await
        .map_err(db)?
        .get(0);
    let client = yeomna_store::connect(dir, port, "yeomna_app", &database)
        .await
        .map_err(|e| VerbError::Internal(format!("the ingest connection failed: {e}")))?;
    PgSink::new(client, graph)
        .await
        .map_err(|e| VerbError::Internal(format!("the sink could not bind the graph: {e}")))
}

fn pipeline_error(e: yeomna_pipeline::PipelineError) -> VerbError {
    VerbError::Internal(format!("ingest failed: {e}"))
}

/// `codebase.ingest`: a tree of source into the graph (FR1).
pub async fn codebase_ingest(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    r: &IngestRequest,
) -> Result<Value, VerbError> {
    let root = readable_tree(&r.path)?;
    let sink = sink_for(s, endpoint, &r.graph).await?;
    let config = CodebaseConfig {
        overwrite: r.overwrite,
        ..Default::default()
    };
    let summary = ingest_codebase(root, &sink, &TokenChunking::default(), None, &config)
        .await
        .map_err(pipeline_error)?;
    Ok(json!({
        "graph": r.graph,
        "path": r.path,
        "files_seen": summary.files_seen,
        "files_written": summary.files_written,
        "files_skipped": summary.files_skipped,
        "files_failed": summary.files_failed,
        "files_oversized": summary.files_oversized,
        "symbols_written": summary.symbols_written,
        "edges_written": summary.edges_written,
        "edges_rejected": summary.edges_rejected,
        "chunks_written": summary.chunks_written,
        "embeddings_written": summary.embeddings_written,
    }))
}

/// `ingest`: documents, then the `conforms` links (FR2).
///
/// The conforms pass runs here because a document ingest is where a
/// corpus's claims arrive, and the links from source to claim are what
/// make them reachable. It walks source rather than documents, so on a
/// documents-only tree it finds nothing and says so.
pub async fn ingest(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    r: &IngestRequest,
) -> Result<Value, VerbError> {
    let root = readable_tree(&r.path)?;
    let sink = sink_for(s, endpoint, &r.graph).await?;
    let config = DocumentsConfig {
        overwrite: r.overwrite,
        ..Default::default()
    };
    let docs = ingest_documents(
        root,
        &sink,
        &NativeExtractor::new(),
        &TokenChunking::default(),
        &config,
    )
    .await
    .map_err(pipeline_error)?;
    let links = link_conforms(root, &sink, &config)
        .await
        .map_err(pipeline_error)?;
    Ok(json!({
        "graph": r.graph,
        "path": r.path,
        "documents": {
            "files_seen": docs.files_seen,
            "files_written": docs.files_written,
            "files_skipped": docs.files_skipped,
            "files_failed": docs.files_failed,
            "files_oversized": docs.files_oversized,
            "collisions": docs.collisions,
            "chunks_written": docs.chunks_written,
            "blocks_seen": docs.blocks_seen,
            "blocks_refused": docs.blocks_refused,
            "nodes_declared": docs.nodes_declared,
            "nodes_fused": docs.nodes_fused,
            "placeholders": docs.placeholders,
            "edges_declared": docs.edges_declared,
            "edges_rejected": docs.edges_rejected,
            "duplicates": docs.duplicates,
            "relations": docs.relations,
            "refusals": docs.refusals,
        },
        "conforms": {
            "files_scanned": links.files_scanned,
            "files_skipped": links.files_skipped,
            "headers_seen": links.headers_seen,
            "malformed": links.malformed,
            "duplicates": links.duplicates,
            "edges_written": links.edges_written,
            "edges_rejected": links.edges_rejected,
            "unresolved": links.unresolved,
            "refusals": links.refusals,
        },
    }))
}

/// `codebase.drift`: does the graph still match the source (D2, FR4).
///
/// Writes nothing. An operator about to trust an answer from the graph
/// asks this first, and a caller that finds `missing` non-empty is
/// looking at what `retire` exists to sweep.
pub async fn codebase_drift(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    r: &DriftRequest,
) -> Result<Value, VerbError> {
    let root = readable_tree(&r.path)?;
    let sink = sink_for(s, endpoint, &r.graph).await?;
    let summary = run_drift(root, &sink, &CodebaseConfig::default())
        .await
        .map_err(pipeline_error)?;
    Ok(json!({
        "graph": r.graph,
        "path": r.path,
        "clean": summary.clean(),
        "files_seen": summary.files_seen,
        "unchanged": summary.unchanged,
        "changed": summary.changed,
        "new": summary.new,
        "missing": summary.missing,
        "unreadable": summary.unreadable,
    }))
}

/// How many offending keys `validate` shows per check. Enough to start
/// looking, bounded so a broken graph does not answer with itself.
const SAMPLE: i64 = 20;

/// `codebase.validate`: the invariants the constraints cannot express
/// (FR5).
///
/// The schema already refuses a missing basis or analyzer, a duplicate
/// edge identity, an unknown node kind, a relation outside its
/// partition's shape, and a dangling reference. What it cannot say is
/// that an edge's endpoints belong to the same graph as the edge,
/// because the foreign keys point at `nodes(id)` and know nothing about
/// `graph_id`. A traversal over such an edge walks out of its own
/// graph, which is the failure this verb exists to find.
pub async fn codebase_validate(s: &Exec<'_>, r: &GraphScoped) -> Result<Value, VerbError> {
    let g: i64 = s
        .client()
        .query_opt("SELECT id FROM graphs WHERE name = $1", &[&r.graph])
        .await
        .map_err(db)?
        .map(|row| row.get(0))
        .ok_or_else(|| VerbError::NotFound(format!("no graph named {:?}", r.graph)))?;

    let cross_graph = s
        .client()
        .query(
            "SELECT src.natural_key || ' -> ' || dst.natural_key
             FROM edges e
             JOIN nodes src ON src.id = e.src_id
             JOIN nodes dst ON dst.id = e.dst_id
             WHERE e.graph_id = $1
               AND (src.graph_id <> e.graph_id OR dst.graph_id <> e.graph_id)
             ORDER BY 1 LIMIT $2",
            &[&g, &SAMPLE],
        )
        .await
        .map_err(db)?;
    let cross_graph_total: i64 = s
        .client()
        .query_one(
            "SELECT count(*) FROM edges e
             JOIN nodes src ON src.id = e.src_id
             JOIN nodes dst ON dst.id = e.dst_id
             WHERE e.graph_id = $1
               AND (src.graph_id <> e.graph_id OR dst.graph_id <> e.graph_id)",
            &[&g],
        )
        .await
        .map_err(db)?
        .get(0);

    let stray_symbols = s
        .client()
        .query_one(
            "SELECT count(*) FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             WHERE n.graph_id = $1
               AND EXISTS (
                 SELECT 1 FROM unnest(c.symbol_ids) sid
                 JOIN nodes s2 ON s2.id = sid
                 WHERE s2.graph_id <> $1)",
            &[&g],
        )
        .await
        .map_err(db)?;

    let models: Vec<String> = s
        .client()
        .query(
            "SELECT DISTINCT em.model FROM embeddings em
             JOIN chunks c ON c.id = em.chunk_id
             JOIN nodes n ON n.id = c.node_id
             WHERE n.graph_id = $1 ORDER BY 1",
            &[&g],
        )
        .await
        .map_err(db)?
        .iter()
        .map(|row| row.get(0))
        .collect();

    let incomplete_files = s
        .client()
        .query(
            "SELECT natural_key FROM nodes
             WHERE graph_id = $1 AND kind = 'file'
               AND (payload->>'path' IS NULL OR payload->>'symbol_hash' IS NULL)
             ORDER BY 1 LIMIT $2",
            &[&g, &SAMPLE],
        )
        .await
        .map_err(db)?;

    let stray: i64 = stray_symbols.get(0);
    let incomplete: Vec<String> = incomplete_files.iter().map(|row| row.get(0)).collect();
    let ok = cross_graph_total == 0 && stray == 0 && incomplete.is_empty() && models.len() <= 1;
    Ok(json!({
        "graph": r.graph,
        "ok": ok,
        "sample_limit": SAMPLE,
        "cross_graph_edges": {
            "count": cross_graph_total,
            "sample": cross_graph.iter().map(|row| row.get::<_, String>(0)).collect::<Vec<_>>(),
        },
        "chunks_naming_foreign_symbols": stray,
        "embedding_models": models,
        "file_nodes_missing_path_or_hash": incomplete,
    }))
}
