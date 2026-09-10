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
use crate::execute::Exec;
use crate::verb::{
    CheckRequest, CountRequest, GraphScoped, KindKey, ListRequest, OrientRequest, QueryRequest,
    RecentRequest, StatsRequest,
};

/// The node kinds the schema's CHECK constraint admits. Kept here as the
/// gate, and identical to the DDL by construction: a kind outside this set
/// could never match a row, so accepting it would return an empty answer
/// where a refusal is the truth.
const KINDS: [&str; 6] = ["file", "module", "type", "callable", "value", "document"];

/// The most rows any paged read returns, whatever was asked for. Reported
/// back as the applied limit rather than the requested one.
pub(crate) const MAX_PAGE: u32 = 1000;

pub(crate) fn check_kind(kind: &str) -> Result<(), VerbError> {
    if KINDS.contains(&kind) {
        return Ok(());
    }
    Err(VerbError::InvalidArgs(format!(
        "unknown kind {kind:?}, expected one of {}",
        KINDS.join(", ")
    )))
}

fn db(e: tokio_postgres::Error) -> VerbError {
    // `tokio_postgres::Error`'s own Display is "db error" and nothing else,
    // so the server's message lives one level down the source chain. An
    // operator reading "internal: store error: db error" learns nothing,
    // which is worse than useless in an appliance whose only window on the
    // store is this envelope.
    let detail = std::error::Error::source(&e)
        .map(|src| src.to_string())
        .unwrap_or_else(|| e.to_string());
    VerbError::Internal(format!("store error: {detail}"))
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
pub async fn orient(s: &Exec<'_>, r: &OrientRequest) -> Result<Value, VerbError> {
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
        // The whole cohort, not only the model (R26, spec 022). Two
        // vectors from the same model at another revision or under another
        // LoRA adapter are incomparable, so a reader deciding whether a
        // graph can be searched with a given query vector needs all three.
        // More than one row here means the graph holds vectors that cannot
        // be ranked against each other, which is worth being able to see.
        let cohorts: Vec<Value> = s
            .client()
            .query(
                "SELECT e.model, e.model_revision, e.task, count(*)
                   FROM embeddings e
                   JOIN chunks c ON c.id = e.chunk_id
                   JOIN nodes n ON n.id = c.node_id
                   JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1
                 GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
                &[&name],
            )
            .await
            .map_err(db)?
            .iter()
            .map(|row| {
                json!({
                    "model": row.get::<_, String>(0),
                    "model_revision": row.get::<_, String>(1),
                    "task": row.get::<_, String>(2),
                    "embeddings": row.get::<_, i64>(3),
                })
            })
            .collect();
        let last: Option<chrono::DateTime<chrono::Utc>> = coverage.get(2);
        graphs.push(json!({
            "graph": name,
            "nodes_by_kind": tally(&nodes),
            "edges_by_relation_and_basis": tally(&edges),
            "chunks": coverage.get::<_, i64>(0),
            "embeddings": coverage.get::<_, i64>(1),
            "embedding_cohorts": cohorts,
            "last_ingest": last.map(|t| t.to_rfc3339()),
        }));
    }
    Ok(json!({
        "schema_version": yeomna_store::SCHEMA_VERSION,
        "graphs": graphs,
    }))
}

/// `status`: does the appliance answer, and what is it.
pub async fn status(s: &Exec<'_>) -> Result<Value, VerbError> {
    let row = s
        .client()
        .query_one(
            "SELECT current_database(), (SELECT count(*) FROM graphs), current_user::text",
            &[],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "store": "answering",
        "database": row.get::<_, String>(0),
        "graphs": row.get::<_, i64>(1),
        // The role this session is executing as, which is how a test, or
        // an operator, proves an escalation did not outlive its verb.
        "role": row.get::<_, String>(2),
        // Who the appliance thinks is calling (V3). The kernel supplied
        // it, no request can change it, and this is where a caller can
        // see it: the audit log records it and no verb reads that.
        "actor": s.actor(),
        "schema_version": yeomna_store::SCHEMA_VERSION,
        "session_graph": s.graph(),
    }))
}

/// `health`: the integrity questions, with the counts behind them.
///
/// Numbers rather than a verdict, because a document with no chunks is
/// normal in one corpus and a defect in another, and the caller knows
/// which it has.
pub async fn health(s: &Exec<'_>) -> Result<Value, VerbError> {
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
pub async fn check(s: &Exec<'_>, r: &CheckRequest) -> Result<Value, VerbError> {
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
pub async fn stats(s: &Exec<'_>, r: &StatsRequest) -> Result<Value, VerbError> {
    let scope = r.graph.as_deref().or_else(|| s.graph());
    // A misspelled name would otherwise answer with zeros, which reads as
    // an empty graph rather than no graph. `orient` and `codebase.stats`
    // already refuse it and this was the odd one out.
    if let Some(g) = scope {
        let exists = s
            .client()
            .query_opt("SELECT 1 FROM graphs WHERE name = $1", &[&g])
            .await
            .map_err(db)?;
        if exists.is_none() {
            return Err(VerbError::NotFound(format!("no graph named {g:?}")));
        }
    }
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
pub async fn codebase_stats(s: &Exec<'_>, r: &GraphScoped) -> Result<Value, VerbError> {
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
pub async fn get(s: &Exec<'_>, r: &KindKey) -> Result<Value, VerbError> {
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
pub async fn list(s: &Exec<'_>, r: &ListRequest) -> Result<Value, VerbError> {
    if let Some(k) = r.kind.as_deref() {
        check_kind(k)?;
    }
    // The applied limit, which is what the response reports: a client
    // paging until it sees a short page would stop early if told 5000 and
    // given 1000.
    let applied = r.limit.min(MAX_PAGE);
    let limit = i64::from(applied);
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
        "limit": applied,
        "offset": r.offset,
    }))
}

/// `count`: how many, by kind when named.
pub async fn count(s: &Exec<'_>, r: &CountRequest) -> Result<Value, VerbError> {
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
pub async fn recent(s: &Exec<'_>, r: &RecentRequest) -> Result<Value, VerbError> {
    let limit = i64::from(r.limit.min(MAX_PAGE));
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
pub async fn query(s: &Exec<'_>, r: &QueryRequest) -> Result<Value, VerbError> {
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
    let limit = i64::from(r.limit.min(MAX_PAGE));
    if r.hybrid {
        return hybrid(s, r, limit).await;
    }
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
        // Named, because `rank` means two different things in the two modes:
        // a `ts_rank_cd` score here and a fused reciprocal-rank score under
        // `hybrid`. A caller reading a number should be able to tell which
        // it has without inferring it from the request it sent.
        "ranking": "keyword",
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

/// The RRF constant (spec 023).
///
/// 60, which is the value the reciprocal-rank-fusion literature uses and
/// the one docling-rag used where the ledger's harvest pointer read it. It
/// is compiled in rather than a request field: a caller who can tune the
/// fusion constant is a caller who can make retrieval quality
/// unreproducible, and two callers tuning it differently would be comparing
/// rankings that are not comparable. Whether 60 suits this corpus is a
/// measurement (M1), not a knob.
const RRF_K: f64 = 60.0;

/// The most candidates either source contributes, whatever the limit.
///
/// Every candidate that survives the fusion is joined back to `chunks` for
/// its text, and a chunk's text is what it is, so the depth is what decides
/// how much text is read to return a page of hits. A thousand is generous
/// against a `MAX_PAGE` of a thousand and bounded enough that a caller
/// cannot ask for ten thousand chunk bodies by asking for a thousand hits.
const MAX_CANDIDATES: i64 = 1000;

/// How many rows each source contributes before fusion.
///
/// Wider than `limit`, because fusion reorders. A chunk ranked eleventh by
/// keyword and eleventh by vector fuses higher than one ranked first by
/// keyword and nowhere by vector, and cutting each source at `limit` would
/// have thrown it away before the fusion could find it. Ten times the
/// requested limit, floored, and capped by [`MAX_CANDIDATES`].
fn candidate_depth(limit: i64) -> i64 {
    (limit.saturating_mul(10)).clamp(50, MAX_CANDIDATES)
}

/// What a graph's vectors are, and how much of it has them.
struct Cohort {
    model: String,
    revision: String,
    task: String,
    /// Chunks in the slice being searched, and how many carry a vector. A
    /// graph is often partly embedded, because a document over the
    /// embedder's ceiling is chunked and not embedded (PRD D5), and a chunk
    /// with no vector is invisible to the vector half. Reported, so a
    /// `vector_rank` of null can be told from a chunk that had no vector to
    /// be ranked by.
    chunks: i64,
    embedded: i64,
}

/// The cohort a graph's vectors belong to, or why there is no single one.
///
/// Read from the rows rather than from config, because the rows are what
/// the vectors are in. R26 put these three columns there for this call.
///
/// Scoped by `kind` as well as by graph, because both halves of the fusion
/// are. A graph whose documents are embedded at one task and whose files are
/// embedded at another holds two cohorts, and a query naming one kind is
/// asking about one of them: checking the whole graph would refuse a query
/// that is perfectly answerable, and worse, a graph where only the
/// unqueried kind is embedded would pass the check and then fuse against an
/// empty vector half.
async fn corpus_cohort(
    s: &Exec<'_>,
    graph: &str,
    kind: Option<&String>,
) -> Result<Cohort, VerbError> {
    let coverage = s
        .client()
        .query_one(
            "SELECT count(*), count(e.chunk_id)
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             LEFT JOIN embeddings e ON e.chunk_id = c.id
             WHERE g.name = $1 AND ($2::text IS NULL OR n.kind = $2)",
            &[&graph, &kind],
        )
        .await
        .map_err(db)?;
    let rows = s
        .client()
        .query(
            "SELECT DISTINCT e.model, e.model_revision, e.task
             FROM embeddings e
             JOIN chunks c ON c.id = e.chunk_id
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND ($2::text IS NULL OR n.kind = $2)
             ORDER BY 1, 2, 3",
            &[&graph, &kind],
        )
        .await
        .map_err(db)?;
    match rows.len() {
        // Not a fallback to keyword ranking. A caller who asked for hybrid
        // and silently got keyword has no way to learn that the vectors it
        // was ranking against do not exist, which is the same reason spec
        // 012 made this flag refuse rather than be ignored.
        0 if coverage.get::<_, i64>(0) == 0 => Err(VerbError::NotFound(format!(
            "graph {graph:?} has no chunks to rank{}. Ingest it, then ask again",
            kind.map(|k| format!(" of kind {k:?}")).unwrap_or_default()
        ))),
        0 => Err(VerbError::NotFound(format!(
            "graph {graph:?} has chunks and no embeddings, so there is nothing to rank \
             against. Ingest it with embed: true, then ask again"
        ))),
        1 => Ok(Cohort {
            model: rows[0].get(0),
            revision: rows[0].get(1),
            task: rows[0].get(2),
            chunks: coverage.get(0),
            embedded: coverage.get(1),
        }),
        n => {
            let named: Vec<String> = rows
                .iter()
                .map(|row| {
                    format!(
                        "{} @ {} / {}",
                        row.get::<_, String>(0),
                        row.get::<_, String>(1),
                        row.get::<_, String>(2)
                    )
                })
                .collect();
            Err(VerbError::InvalidArgs(format!(
                "graph {graph:?} holds {n} embedding cohorts, and no one query vector is \
                 comparable to all of them: {}. Re-ingest the graph under one cohort, or \
                 query without hybrid",
                named.join(", ")
            )))
        }
    }
}

/// `query --hybrid`: reciprocal rank fusion over keyword and vector, in one
/// statement (spec 023, PRD-embedder Phase 3).
///
/// The store PRD has claimed since it was drafted that "hybrid is one
/// statement", against a reference whose search verb was four round trips
/// plus a Rust loop. This is that claim made true or not: one statement,
/// one round trip, two CTEs and a full outer join.
///
/// The query vector is computed here rather than accepted from the caller
/// (PRD D7). A `vector` field on the request would be a way to reach vector
/// search without calling `embed.text`, which is a side door around a verb
/// and would falsify T3 the moment `embed.text` refused anything. It also
/// makes the cohort check possible at all, since a caller-supplied vector
/// carries no task.
async fn hybrid(s: &Exec<'_>, r: &QueryRequest, limit: i64) -> Result<Value, VerbError> {
    // Hybrid needs a graph, because a cohort is a property of one. Across
    // an unscoped database there could be as many cohorts as graphs.
    let Some(graph) = s.graph() else {
        return Err(VerbError::InvalidArgs(
            "hybrid ranking needs a graph, because the cohort its vectors belong to is a \
             property of one. Scope the session or name a graph"
                .into(),
        ));
    };
    let cohort = corpus_cohort(s, graph, r.kind.as_ref()).await?;
    let vector = crate::embed::query_vector_literal(
        s,
        &r.search_text,
        &cohort.model,
        &cohort.revision,
        &cohort.task,
    )
    .await?;
    let depth = candidate_depth(limit);

    // `keyword` ranks by ts_rank_cd exactly as the non-hybrid path does, so
    // the two modes cannot disagree about what a keyword match is worth.
    // `vector` ranks by cosine distance.
    //
    // **The vector half does not use `embeddings_hnsw`, and M5 is why.** The
    // graph filter reaches `embeddings` only through
    // `chunks -> nodes -> graphs`, so the planner drives from the graph side
    // and the vector index cannot supply the ordering. Every embedding in
    // the graph is fetched and distances are computed over all of them.
    // Measured on this cluster: 4.8 ms and 9,541 buffers over 1,275
    // embedded chunks, against 0.28 ms and 368 buffers for the same nearest
    // search with no graph filter, where the index does run. The ranking is
    // exact, which brute force always is, and the cost is linear in the
    // graph's embedding count. Fixing it wants a filter column on
    // `embeddings`, which is a schema change and a re-ingest, so it is
    // measured and named rather than smuggled into this spec. Both halves
    // still order and limit inside a subquery, which is what lets the sort
    // be a top-N heapsort rather than a full sort, and what the index would
    // need if the filter ever reaches it.
    //
    // The full outer join keeps a chunk that appeared in only one source,
    // which is the case fusion exists for, and COALESCE on the key is what
    // makes the join's own row identity work.
    // Each rank window orders by the key its subquery ordered on. An empty
    // `OVER ()` numbers rows in whatever order they arrive, which happens
    // to be the subquery's today and is not promised to be, and a rank is
    // what the fusion score is built from, so it cannot rest on a plan
    // detail. The re-sort is over at most `candidate_depth` rows.
    let rows = s
        .client()
        .query(
            "WITH keyword AS (
                 SELECT chunk_id,
                        row_number() OVER (ORDER BY score DESC, chunk_id) AS rank
                 FROM (
                     SELECT c.id AS chunk_id,
                            ts_rank_cd(c.tsv, websearch_to_tsquery('english', $1)) AS score
                     FROM chunks c
                     JOIN nodes n ON n.id = c.node_id
                     JOIN graphs g ON g.id = n.graph_id
                     WHERE g.name = $2
                       AND ($3::text IS NULL OR n.kind = $3)
                       AND c.tsv @@ websearch_to_tsquery('english', $1)
                     ORDER BY score DESC, c.id
                     LIMIT $4
                 ) ranked
             ),
             vector AS (
                 SELECT chunk_id,
                        row_number() OVER (ORDER BY distance, chunk_id) AS rank
                 FROM (
                     SELECT c.id AS chunk_id,
                            e.vec <=> $5::text::halfvec AS distance
                     FROM embeddings e
                     JOIN chunks c ON c.id = e.chunk_id
                     JOIN nodes n ON n.id = c.node_id
                     JOIN graphs g ON g.id = n.graph_id
                     WHERE g.name = $2
                       AND ($3::text IS NULL OR n.kind = $3)
                     ORDER BY distance, c.id
                     LIMIT $4
                 ) nearest
             ),
             fused AS (
                 SELECT COALESCE(k.chunk_id, v.chunk_id) AS chunk_id,
                        k.rank AS text_rank,
                        v.rank AS vector_rank,
                        -- Every cast is explicit. Left to inference,
                        -- `1.0 / ($6 + k.rank)` makes Postgres want numeric
                        -- for $6 where the client is sending float8, and
                        -- the failure is a serialization error naming a
                        -- parameter number rather than a type.
                        COALESCE(1.0::float8 / ($6::float8 + k.rank::float8), 0::float8)
                          + COALESCE(1.0::float8 / ($6::float8 + v.rank::float8), 0::float8)
                          AS score
                 FROM keyword k
                 FULL OUTER JOIN vector v ON v.chunk_id = k.chunk_id
             ),
             -- The page is taken before the chunk bodies are joined, so a
             -- deep candidate set does not cost a chunk body per candidate
             -- to return a handful of hits.
             top AS (
                 SELECT chunk_id, text_rank, vector_rank, score
                 FROM fused
                 ORDER BY score DESC, chunk_id
                 LIMIT $7
             )
             SELECT g.name, n.natural_key, n.kind, c.chunk_index, c.text,
                    t.score, t.text_rank, t.vector_rank
             FROM top t
             JOIN chunks c ON c.id = t.chunk_id
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             ORDER BY t.score DESC, n.natural_key, c.chunk_index",
            &[
                &r.search_text,
                &graph,
                &r.kind,
                &depth,
                &vector,
                &RRF_K,
                &limit,
            ],
        )
        .await
        .map_err(db)?;

    Ok(json!({
        "search_text": r.search_text,
        "ranking": "hybrid",
        // Which cohort answered, so a caller reading a surprising result can
        // see what the ranking was against.
        "cohort": {
            "model": cohort.model,
            "model_revision": cohort.revision,
            "corpus_task": cohort.task,
            "query_task": crate::embed::query_task_for(&cohort.task),
        },
        // How much of the slice the vector half could see. When `embedded`
        // is short of `chunks`, some chunk's `vector_rank` is null because
        // it has no vector rather than because it ranked below the
        // candidate depth, and the two are otherwise indistinguishable.
        "coverage": {
            "chunks": cohort.chunks,
            "embedded": cohort.embedded,
        },
        // `candidate_depth` is what each source really contributed, which
        // is why `hnsw.ef_search` is set to it rather than left at its
        // default: a depth in the response that the index did not honor
        // would be worse than no depth at all.
        "fusion": { "method": "rrf", "k": RRF_K, "candidate_depth": depth },
        "hits": rows.iter().map(|row| json!({
            "graph": row.get::<_, String>(0),
            "key": row.get::<_, String>(1),
            "kind": row.get::<_, String>(2),
            "chunk_index": row.get::<_, i32>(3),
            "text": row.get::<_, String>(4),
            "rank": row.get::<_, f64>(5),
            // Absent when the chunk appeared in only one source, which is
            // the thing worth being able to see in a fused result.
            "text_rank": row.get::<_, Option<i64>>(6),
            "vector_rank": row.get::<_, Option<i64>>(7),
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EC-3: fusion reorders, so each source has to contribute more rows
    /// than the caller asked for. A chunk ranked eleventh in each source
    /// fuses above one ranked first in a single source, and cutting each
    /// source at `limit` would throw it away before the fusion could find
    /// it. Observed on the real corpus: a chunk at keyword rank 22 and
    /// vector rank 10 landed sixth in a fused top six.
    #[test]
    fn candidates_are_wider_than_the_limit_because_fusion_reorders() {
        for limit in [1i64, 5, 10, 20] {
            assert!(
                candidate_depth(limit) > limit,
                "limit {limit} takes a wider slice, got {}",
                candidate_depth(limit)
            );
        }
        // Floored, so a limit of 1 still has something to fuse.
        assert_eq!(candidate_depth(1), 50);
        // Capped, because every candidate costs a chunk body at the end.
        assert_eq!(candidate_depth(i64::from(MAX_PAGE)), MAX_CANDIDATES);
        // And no overflow at the edge, which `saturating_mul` is for.
        assert_eq!(candidate_depth(i64::MAX), MAX_CANDIDATES);
    }

    /// FR9: the fusion constant is compiled in. A test that reads it is the
    /// cheapest guard against it quietly becoming configurable.
    #[test]
    fn the_fusion_constant_is_the_literature_value() {
        assert_eq!(RRF_K, 60.0);
    }
}
