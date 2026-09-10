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
use crate::verb::{DriftRequest, DropScoped, GraphScoped, IngestRequest, RetireRequest};

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
        "unassessed": summary.unassessed,
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

/// `codebase.retire`: sweep what the source no longer has (D3).
///
/// The first destructive ingestion verb, and the one the charter names
/// when it says T3 is false. What makes it safe is that it does not
/// decide for itself what is gone: it asks `drift`, which walks the
/// tree the way an ingest does, and takes only what drift calls
/// `missing`. A file drift could not assess is present as far as this
/// verb is concerned, because deleting the graph's knowledge of source
/// that is sitting right there is the worse error (FR3).
pub async fn codebase_retire(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    r: &RetireRequest,
) -> Result<Value, VerbError> {
    if !r.force {
        return Err(VerbError::Denied(format!(
            "retiring {:?} under prefix {:?} deletes the graph's record of files the tree no longer has, pass force to acknowledge",
            r.graph, r.prefix
        )));
    }
    // EC-6: an empty prefix would mean the whole graph. A destructive
    // verb does not accept a wildcard by omission.
    if r.prefix.trim().is_empty() {
        return Err(VerbError::InvalidArgs(
            "prefix is empty, which would name every file in the graph".into(),
        ));
    }
    // EC-2: a tree that cannot be walked would make every file look
    // gone. A missing tree is not license to empty a graph.
    let root = readable_tree(&r.path)?;
    let sink = sink_for(s, endpoint, &r.graph).await?;
    let assessment = run_drift(root, &sink, &CodebaseConfig::default())
        .await
        .map_err(pipeline_error)?;

    let gone: Vec<String> = assessment
        .missing
        .iter()
        .filter(|p| p.starts_with(&r.prefix))
        .cloned()
        .collect();
    if gone.is_empty() {
        return Ok(json!({
            "graph": r.graph,
            "prefix": r.prefix,
            "retired": [],
            "swept": {"nodes": 0, "edges": 0, "chunks": 0, "embeddings": 0, "log_entries": 0},
        }));
    }

    let keys: Vec<String> = gone.iter().map(|p| yeomna_keys::file_key(p)).collect();
    // Measured and deleted as sub-statements of one WITH, so the counts
    // an operator reads are the counts that went (FR6, spec 013's
    // pattern). The family is the file node plus every node that names
    // it as parent, since a symbol names its file by derived key rather
    // than by foreign key and would otherwise be left standing.
    let counts = s
        .client()
        .query_one(
            "WITH family AS (
               SELECT n.id FROM nodes n
               JOIN graphs g ON g.id = n.graph_id
               WHERE g.name = $1
                 AND (n.natural_key = ANY($2) OR n.payload->>'file_key' = ANY($2))
             ),
             measured AS (
               SELECT
                 (SELECT count(*) FROM family) AS nodes,
                 (SELECT count(*) FROM edges e
                    WHERE e.src_id IN (SELECT id FROM family)
                       OR e.dst_id IN (SELECT id FROM family)) AS edges,
                 (SELECT count(*) FROM chunks c
                    WHERE c.node_id IN (SELECT id FROM family)) AS chunks,
                 (SELECT count(*) FROM embeddings em JOIN chunks c ON c.id = em.chunk_id
                    WHERE c.node_id IN (SELECT id FROM family)) AS embeddings,
                 (SELECT count(*) FROM node_log l
                    WHERE l.node_id IN (SELECT id FROM family)) AS log_entries
             ),
             deleted AS (DELETE FROM nodes WHERE id IN (SELECT id FROM family))
             SELECT nodes, edges, chunks, embeddings, log_entries FROM measured",
            &[&r.graph, &keys],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "graph": r.graph,
        "prefix": r.prefix,
        "retired": gone,
        "swept": {
            "nodes": counts.get::<_, i64>(0),
            "edges": counts.get::<_, i64>(1),
            "chunks": counts.get::<_, i64>(2),
            "embeddings": counts.get::<_, i64>(3),
            "log_entries": counts.get::<_, i64>(4),
        },
    }))
}

/// `codebase.prune`: sweep the orphans (D3).
///
/// Chiefly the symbol nodes whose declaring file node is absent, which
/// is the class a half-finished retire leaves and the class a
/// hand-written insert can create at any time. A symbol whose
/// `file_key` names a file node in another graph is not an orphan here
/// and is left alone (EC-5), which is the same cross-graph discipline
/// `validate` reports on.
pub async fn codebase_prune(s: &Exec<'_>, r: &DropScoped) -> Result<Value, VerbError> {
    if !r.force {
        return Err(VerbError::Denied(format!(
            "pruning {:?} deletes nodes whose declaring file is no longer in the graph, pass force to acknowledge",
            r.graph
        )));
    }
    let g: i64 = s
        .client()
        .query_opt("SELECT id FROM graphs WHERE name = $1", &[&r.graph])
        .await
        .map_err(db)?
        .map(|row| row.get(0))
        .ok_or_else(|| VerbError::NotFound(format!("no graph named {:?}", r.graph)))?;

    // An orphan is judged inside its own graph. Widening the parent
    // lookup across graphs would let a file node anywhere mask a real
    // orphan here, and that is not hypothetical: `file_key` is derived
    // from the path alone, so two graphs over the same tree hold the
    // same keys and each would hide the other's orphans.
    //
    // The sample comes from the DELETE's own RETURNING rather than a
    // read taken beforehand, so it names what went rather than what was
    // there a moment earlier, which is the same snapshot discipline the
    // counts follow.
    let counts = s
        .client()
        .query_one(
            "WITH orphans AS (
               SELECT n.id FROM nodes n
               WHERE n.graph_id = $1
                 AND n.payload ? 'file_key'
                 AND NOT EXISTS (
                   SELECT 1 FROM nodes f
                   WHERE f.graph_id = n.graph_id
                     AND f.natural_key = n.payload->>'file_key')
             ),
             measured AS (
               SELECT
                 (SELECT count(*) FROM orphans) AS nodes,
                 (SELECT count(*) FROM edges e
                    WHERE e.src_id IN (SELECT id FROM orphans)
                       OR e.dst_id IN (SELECT id FROM orphans)) AS edges,
                 (SELECT count(*) FROM chunks c
                    WHERE c.node_id IN (SELECT id FROM orphans)) AS chunks,
                 (SELECT count(*) FROM node_log l
                    WHERE l.node_id IN (SELECT id FROM orphans)) AS log_entries
             ),
             deleted AS (
               DELETE FROM nodes WHERE id IN (SELECT id FROM orphans)
               RETURNING natural_key
             )
             SELECT m.nodes, m.edges, m.chunks, m.log_entries,
                    COALESCE(
                      (SELECT array_agg(k) FROM
                         (SELECT natural_key AS k FROM deleted ORDER BY 1 LIMIT $2) s),
                      '{}'
                    ) AS sample
             FROM measured m",
            &[&g, &SAMPLE],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "graph": r.graph,
        "sample_limit": SAMPLE,
        "sample": counts.get::<_, Vec<String>>(4),
        "swept": {
            "nodes": counts.get::<_, i64>(0),
            "edges": counts.get::<_, i64>(1),
            "chunks": counts.get::<_, i64>(2),
            "log_entries": counts.get::<_, i64>(3),
        },
    }))
}
