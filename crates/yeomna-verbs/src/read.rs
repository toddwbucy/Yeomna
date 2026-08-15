//! The eleven read verbs, each owning the SQL it emits.
//!
//! `yeomna-verbs` is the second crate the no-SQL lint allows to hold SQL,
//! and this is why. Every value is bound. The one place a caller's input
//! could reach SQL structure is `kind`, and it is checked against the
//! schema's own CHECK set before it goes anywhere (FR 5), so an unknown
//! kind is a typed refusal rather than an empty result that looks like an
//! answer.

use serde_json::{Value, json};
use tokio_postgres::Row;

use crate::error::VerbError;
use crate::execute::Session;
use crate::verb::{
    CheckRequest, CountRequest, GraphScoped, KindKey, ListRequest, OrientRequest, QueryRequest,
    RecentRequest, StatsRequest,
};

/// The node kinds the schema's CHECK constraint admits. Kept here as the
/// gate, and identical to the DDL by construction: a kind outside this set
/// could never match a row, so accepting it would return an empty answer
/// where a refusal is the truth.
const KINDS: [&str; 6] = ["file", "module", "type", "callable", "value", "document"];

fn check_kind(kind: &str) -> Result<(), VerbError> {
    if KINDS.contains(&kind) {
        return Ok(());
    }
    Err(VerbError::InvalidArgs(format!(
        "unknown kind {kind:?}, expected one of {}",
        KINDS.join(", ")
    )))
}

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

/// Group rows of `(label, count)` into an object.
fn tally(rows: &[Row]) -> Value {
    let mut out = serde_json::Map::new();
    for r in rows {
        out.insert(r.get::<_, String>(0), json!(r.get::<_, i64>(1)));
    }
    Value::Object(out)
}

/// `schema.version`: what schema this binary ships, per spec 012's ruling.
pub fn schema_version() -> Result<Value, VerbError> {
    Ok(json!({ "version": yeomna_store::SCHEMA_VERSION }))
}

/// `orient`: the per-graph survey V-Q3 ruled.
///
/// What a graph is about and where it stands, as structured facts. Any
/// narrative belongs to whatever is reading, and any welcome text belongs
/// to the deployment's config file (H10).
pub async fn orient(s: &Session, r: &OrientRequest) -> Result<Value, VerbError> {
    let names: Vec<String> = match r.graph.as_deref().or_else(|| s.graph()) {
        Some(g) => {
            let exists = s
                .client()
                .query_opt("SELECT 1 FROM graphs WHERE name = $1", &[&g])
                .await
                .map_err(db)?;
            // EC-3: a graph that is not there is not the same as a graph
            // that is empty, and the survey of an empty graph is zeros.
            if exists.is_none() {
                return Err(VerbError::NotFound(format!("no graph named {g:?}")));
            }
            vec![g.to_string()]
        }
        None => s
            .client()
            .query("SELECT name FROM graphs ORDER BY name", &[])
            .await
            .map_err(db)?
            .iter()
            .map(|row| row.get(0))
            .collect(),
    };

    let mut graphs = Vec::new();
    for name in names {
        let nodes = s
            .client()
            .query(
                "SELECT n.kind, count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1 GROUP BY 1 ORDER BY 1",
                &[&name],
            )
            .await
            .map_err(db)?;
        let edges = s
            .client()
            .query(
                "SELECT e.relation || '/' || e.basis::text, count(*)
                 FROM edges e JOIN graphs g ON g.id = e.graph_id
                 WHERE g.name = $1 GROUP BY 1 ORDER BY 1",
                &[&name],
            )
            .await
            .map_err(db)?;
        let coverage = s
            .client()
            .query_one(
                "SELECT
                   (SELECT count(*) FROM chunks c
                      JOIN nodes n ON n.id = c.node_id
                      JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1),
                   (SELECT count(*) FROM embeddings e
                      JOIN chunks c ON c.id = e.chunk_id
                      JOIN nodes n ON n.id = c.node_id
                      JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1),
                   (SELECT max(n.ingested_at) FROM nodes n
                      JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1)",
                &[&name],
            )
            .await
            .map_err(db)?;
        let models: Vec<String> = s
            .client()
            .query(
                "SELECT DISTINCT e.model FROM embeddings e
                   JOIN chunks c ON c.id = e.chunk_id
                   JOIN nodes n ON n.id = c.node_id
                   JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1 ORDER BY 1",
                &[&name],
            )
            .await
            .map_err(db)?
            .iter()
            .map(|row| row.get(0))
            .collect();
        let last: Option<chrono::DateTime<chrono::Utc>> = coverage.get(2);
        graphs.push(json!({
            "graph": name,
            "nodes_by_kind": tally(&nodes),
            "edges_by_relation_and_basis": tally(&edges),
            "chunks": coverage.get::<_, i64>(0),
            "embeddings": coverage.get::<_, i64>(1),
            "embedding_models": models,
            "last_ingest": last.map(|t| t.to_rfc3339()),
        }));
    }
    Ok(json!({
        "schema_version": yeomna_store::SCHEMA_VERSION,
        "graphs": graphs,
    }))
}

/// `status`: does the appliance answer, and what is it.
pub async fn status(s: &Session) -> Result<Value, VerbError> {
    let row = s
        .client()
        .query_one(
            "SELECT current_database(), (SELECT count(*) FROM graphs)",
            &[],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "store": "answering",
        "database": row.get::<_, String>(0),
        "graphs": row.get::<_, i64>(1),
        "schema_version": yeomna_store::SCHEMA_VERSION,
        "session_graph": s.graph(),
    }))
}

/// `health`: the integrity questions, with the counts behind them.
///
/// Numbers rather than a verdict, because a document with no chunks is
/// normal in one corpus and a defect in another, and the caller knows
/// which it has.
pub async fn health(s: &Session) -> Result<Value, VerbError> {
    let row = s
        .client()
        .query_one(
            "SELECT
               (SELECT count(*) FROM nodes n
                  WHERE n.kind IN ('document', 'file')
                    AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.node_id = n.id)),
               (SELECT count(*) FROM chunks c
                  WHERE NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.chunk_id = c.id)),
               (SELECT count(*) FROM nodes),
               (SELECT count(*) FROM chunks)",
            &[],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "documents_without_chunks": row.get::<_, i64>(0),
        "chunks_without_embeddings": row.get::<_, i64>(1),
        "nodes": row.get::<_, i64>(2),
        "chunks": row.get::<_, i64>(3),
    }))
}

/// `check`: does this key exist, and where.
pub async fn check(s: &Session, r: &CheckRequest) -> Result<Value, VerbError> {
    let rows = s
        .client()
        .query(
            "SELECT g.name, n.kind FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE n.natural_key = $1 ORDER BY g.name",
            &[&r.key],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "key": r.key,
        "exists": !rows.is_empty(),
        "found_in": rows.iter().map(|row| json!({
            "graph": row.get::<_, String>(0),
            "kind": row.get::<_, String>(1),
        })).collect::<Vec<_>>(),
    }))
}

/// `stats`: row counts, scoped to a graph when one is named.
pub async fn stats(s: &Session, r: &StatsRequest) -> Result<Value, VerbError> {
    let scope = r.graph.as_deref().or_else(|| s.graph());
    let row = s
        .client()
        .query_one(
            "SELECT
               (SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
                  WHERE $1::text IS NULL OR g.name = $1),
               (SELECT count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
                  WHERE $1::text IS NULL OR g.name = $1),
               (SELECT count(*) FROM chunks c JOIN nodes n ON n.id = c.node_id
                  JOIN graphs g ON g.id = n.graph_id WHERE $1::text IS NULL OR g.name = $1),
               (SELECT count(*) FROM embeddings e JOIN chunks c ON c.id = e.chunk_id
                  JOIN nodes n ON n.id = c.node_id JOIN graphs g ON g.id = n.graph_id
                  WHERE $1::text IS NULL OR g.name = $1),
               (SELECT count(*) FROM graphs WHERE $1::text IS NULL OR name = $1)",
            &[&scope],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "scope": scope,
        "nodes": row.get::<_, i64>(0),
        "edges": row.get::<_, i64>(1),
        "chunks": row.get::<_, i64>(2),
        "embeddings": row.get::<_, i64>(3),
        "graphs": row.get::<_, i64>(4),
    }))
}

/// `codebase.stats`: what shape a code graph is in.
pub async fn codebase_stats(s: &Session, r: &GraphScoped) -> Result<Value, VerbError> {
    let symbols = s
        .client()
        .query(
            "SELECT n.kind, count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 GROUP BY 1 ORDER BY 2 DESC",
            &[&r.graph],
        )
        .await
        .map_err(db)?;
    let relations = s
        .client()
        .query(
            "SELECT e.relation, count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = $1 GROUP BY 1 ORDER BY 2 DESC",
            &[&r.graph],
        )
        .await
        .map_err(db)?;
    let analyzers = s
        .client()
        .query(
            "SELECT e.analyzer, count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = $1 GROUP BY 1 ORDER BY 2 DESC",
            &[&r.graph],
        )
        .await
        .map_err(db)?;
    if symbols.is_empty() && relations.is_empty() {
        let exists = s
            .client()
            .query_opt("SELECT 1 FROM graphs WHERE name = $1", &[&r.graph])
            .await
            .map_err(db)?;
        if exists.is_none() {
            return Err(VerbError::NotFound(format!("no graph named {:?}", r.graph)));
        }
    }
    Ok(json!({
        "graph": r.graph,
        "nodes_by_kind": tally(&symbols),
        "edges_by_relation": tally(&relations),
        "analyzers": tally(&analyzers),
    }))
}

/// `get`: one node, by kind and key.
pub async fn get(s: &Session, r: &KindKey) -> Result<Value, VerbError> {
    check_kind(&r.kind)?;
    let rows = s
        .client()
        .query(
            "SELECT g.name, n.natural_key, n.kind, n.payload::text, n.ingested_at
             FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE n.natural_key = $1 AND n.kind = $2
               AND ($3::text IS NULL OR g.name = $3)
             ORDER BY g.name",
            &[&r.key, &r.kind, &s.graph()],
        )
        .await
        .map_err(db)?;
    match rows.len() {
        0 => Err(VerbError::NotFound(format!(
            "no {} named {:?}",
            r.kind, r.key
        ))),
        1 => Ok(node_json(&rows[0])),
        // EC-2: with no session graph a key can live in several graphs.
        // Reporting the ambiguity beats picking one, and beats a
        // NotFound that would be false.
        _ => Err(VerbError::InvalidArgs(format!(
            "{:?} exists in {} graphs ({}), so name one on the session",
            r.key,
            rows.len(),
            rows.iter()
                .map(|row| row.get::<_, String>(0))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn node_json(row: &Row) -> Value {
    let payload: Value = serde_json::from_str(&row.get::<_, String>(3)).unwrap_or(Value::Null);
    let at: chrono::DateTime<chrono::Utc> = row.get(4);
    json!({
        "graph": row.get::<_, String>(0),
        "key": row.get::<_, String>(1),
        "kind": row.get::<_, String>(2),
        "payload": payload,
        "ingested_at": at.to_rfc3339(),
    })
}

/// `list`: nodes by kind, paged, optionally under a parent.
pub async fn list(s: &Session, r: &ListRequest) -> Result<Value, VerbError> {
    if let Some(k) = r.kind.as_deref() {
        check_kind(k)?;
    }
    let limit = i64::from(r.limit.min(1000));
    let offset = i64::from(r.offset);
    let rows = s
        .client()
        .query(
            "SELECT g.name, n.natural_key, n.kind, n.payload::text, n.ingested_at
             FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE ($1::text IS NULL OR n.kind = $1)
               AND ($2::text IS NULL OR g.name = $2)
               AND ($3::text IS NULL OR n.payload->>'file_key' = $3
                    OR n.payload->>'doc_key' = $3)
             ORDER BY n.ingested_at DESC, n.natural_key
             LIMIT $4 OFFSET $5",
            &[&r.kind, &s.graph(), &r.parent, &limit, &offset],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "nodes": rows.iter().map(node_json).collect::<Vec<_>>(),
        "limit": r.limit,
        "offset": r.offset,
    }))
}

/// `count`: how many, by kind when named.
pub async fn count(s: &Session, r: &CountRequest) -> Result<Value, VerbError> {
    if let Some(k) = r.kind.as_deref() {
        check_kind(k)?;
    }
    let n: i64 = s
        .client()
        .query_one(
            "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE ($1::text IS NULL OR n.kind = $1)
               AND ($2::text IS NULL OR g.name = $2)",
            &[&r.kind, &s.graph()],
        )
        .await
        .map_err(db)?
        .get(0);
    Ok(json!({ "count": n, "kind": r.kind, "graph": s.graph() }))
}

/// `recent`: what landed last, which R9's `ingested_at` made answerable.
pub async fn recent(s: &Session, r: &RecentRequest) -> Result<Value, VerbError> {
    let limit = i64::from(r.limit.min(1000));
    let rows = s
        .client()
        .query(
            "SELECT g.name, n.natural_key, n.kind, n.payload::text, n.ingested_at
             FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE $1::text IS NULL OR g.name = $1
             ORDER BY n.ingested_at DESC, n.natural_key
             LIMIT $2",
            &[&s.graph(), &limit],
        )
        .await
        .map_err(db)?;
    Ok(json!({ "nodes": rows.iter().map(node_json).collect::<Vec<_>>() }))
}

/// `query`: full-text search over chunks, ranked by `ts_rank_cd`.
///
/// A1: this is search over text, not a filter engine. There is no
/// caller-supplied field or operator, so there is no allowlist to escape,
/// which is the shape charter section 6 wants.
pub async fn query(s: &Session, r: &QueryRequest) -> Result<Value, VerbError> {
    if r.hybrid {
        return Err(VerbError::Unimplemented(
            "hybrid ranking needs the embedder, H4. Ask again without it".into(),
        ));
    }
    if r.structural {
        return Err(VerbError::Unimplemented(
            "structural ranking needs graph embeddings, H9. Ask again without it".into(),
        ));
    }
    if let Some(k) = r.kind.as_deref() {
        check_kind(k)?;
    }
    if r.search_text.trim().is_empty() {
        return Err(VerbError::InvalidArgs("search_text is empty".into()));
    }
    let limit = i64::from(r.limit.min(1000));
    let rows = s
        .client()
        .query(
            "SELECT g.name, n.natural_key, n.kind, c.chunk_index, c.text,
                    ts_rank_cd(c.tsv, websearch_to_tsquery('english', $1)) AS rank
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE c.tsv @@ websearch_to_tsquery('english', $1)
               AND ($2::text IS NULL OR n.kind = $2)
               AND ($3::text IS NULL OR g.name = $3)
             ORDER BY rank DESC, n.natural_key, c.chunk_index
             LIMIT $4",
            &[&r.search_text, &r.kind, &s.graph(), &limit],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "search_text": r.search_text,
        "hits": rows.iter().map(|row| json!({
            "graph": row.get::<_, String>(0),
            "key": row.get::<_, String>(1),
            "kind": row.get::<_, String>(2),
            "chunk_index": row.get::<_, i32>(3),
            "text": row.get::<_, String>(4),
            "rank": row.get::<_, f32>(5),
        })).collect::<Vec<_>>(),
    }))
}
