//! The mutation verbs and the audit transaction (spec 014).
//!
//! Every function here runs its mutation, its diff-log entries, and the
//! `outcome = 'ok'` mark inside one transaction, so an ok outcome on a
//! write verb is durable if and only if the mutation is (FR5). A failure
//! or a crash between the first statement and the commit rolls all of it
//! back, leaving the attempt row spec 012 committed beforehand to tell
//! the story: NULL for a crash, `failed: <kind>` once the generic pass
//! marks the error.
//!
//! Deletion is the inherited cascade (store PRD): removing a head row
//! sweeps its chunks, embeddings, edges, and log entries through the
//! foreign keys, measured first in the same snapshot so the reported
//! counts are what the cascade removed. The diff log goes with the node
//! it describes, and the audit row is the durable record of the act.

use serde_json::{Value, json};
use tokio_postgres::{Client, Transaction};

use crate::audit::Attempt;
use crate::error::VerbError;
use crate::read::check_kind;
use crate::verb::{EdgeAssertRequest, EdgeRetractRequest, KindKey, PurgeRequest, WriteRequest};

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

/// The session graph, which every mutation requires: writes land
/// somewhere on purpose or not at all (EC-1).
fn session_graph(graph: Option<&str>) -> Result<&str, VerbError> {
    graph.ok_or_else(|| {
        VerbError::InvalidArgs(
            "this verb writes to the session's graph, and the session has none".into(),
        )
    })
}

/// An asserted relation is the caller's vocabulary, bounded to identifier
/// shape and deliberately not enumerated (R18). Mirrors the
/// `edges_asserted` partition CHECK by construction, hyphen included
/// since R19a admitted kebab on the open partitions.
fn check_relation(relation: &str) -> Result<(), VerbError> {
    let ok = !relation.is_empty()
        && relation.len() <= 63
        && relation
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
        && relation
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        return Ok(());
    }
    Err(VerbError::InvalidArgs(format!(
        "relation {relation:?} must be a lowercase identifier (a-z, then a-z, 0-9, _, -), at most 63 bytes"
    )))
}

async fn graph_id(t: &Transaction<'_>, name: &str) -> Result<i64, VerbError> {
    t.query_opt("SELECT id FROM graphs WHERE name = $1", &[&name])
        .await
        .map_err(db)?
        .map(|r| r.get(0))
        .ok_or_else(|| VerbError::NotFound(format!("no graph named {name:?}")))
}

/// Append one entry to a node's history. The diff shapes are the sink's,
/// byte for byte, because one log read by anything that replays history
/// cannot afford two dialects.
async fn append_log(t: &Transaction<'_>, node_id: i64, diff: &Value) -> Result<(), VerbError> {
    t.execute(
        "INSERT INTO node_log (node_id, seq, diff)
         VALUES ($1,
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM node_log WHERE node_id = $1),
                 $2::text::jsonb)",
        &[&node_id, &diff.to_string()],
    )
    .await
    .map_err(db)?;
    Ok(())
}

/// Mark the attempt ok inside the caller's transaction (FR5). The NULL
/// guard keeps the first mark the only mark.
async fn mark_ok(t: &Transaction<'_>, attempt: Attempt) -> Result<(), VerbError> {
    t.execute(
        "UPDATE audit_log SET outcome = 'ok' WHERE id = $1 AND outcome IS NULL",
        &[&attempt.id()],
    )
    .await
    .map_err(db)?;
    Ok(())
}

/// The insert transaction body, separated from its commit so the
/// crash-shaped test below can run every product statement and then
/// drop the transaction where a crash would.
async fn insert_txn(
    t: &Transaction<'_>,
    graph_name: &str,
    attempt: Attempt,
    r: &WriteRequest,
) -> Result<Value, VerbError> {
    let g = graph_id(t, graph_name).await?;
    let row = t
        .query_opt(
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, $2, $3, $4::text::jsonb)
             ON CONFLICT (graph_id, natural_key) DO NOTHING
             RETURNING id",
            &[&g, &r.key, &r.kind, &r.payload.to_string()],
        )
        .await
        .map_err(db)?;
    let Some(row) = row else {
        return Err(VerbError::InvalidArgs(format!(
            "{:?} already exists in graph {graph_name:?}, and update is the verb for changing it",
            r.key
        )));
    };
    let id: i64 = row.get(0);
    append_log(t, id, &json!({"op": "insert", "to": r.payload})).await?;
    mark_ok(t, attempt).await?;
    Ok(json!({
        "graph": graph_name,
        "kind": r.kind,
        "key": r.key,
        "created": true,
    }))
}

/// `insert`: a new node, head plus first log entry (FR1).
pub async fn insert(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &WriteRequest,
) -> Result<Value, VerbError> {
    let graph_name = session_graph(graph)?;
    check_kind(&r.kind)?;
    let t = client.transaction().await.map_err(db)?;
    let out = insert_txn(&t, graph_name, attempt, r).await?;
    t.commit().await.map_err(db)?;
    Ok(out)
}

/// `update`: replace a head payload and log the change (FR2). A payload
/// identical under IS DISTINCT FROM is an audited ok that touches
/// nothing, the R8 discipline held from ingest.
pub async fn update(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &WriteRequest,
) -> Result<Value, VerbError> {
    let graph_name = session_graph(graph)?;
    check_kind(&r.kind)?;
    let t = client.transaction().await.map_err(db)?;
    let g = graph_id(&t, graph_name).await?;
    let payload = r.payload.to_string();
    // FOR UPDATE: under READ COMMITTED an unlocked read can see a payload
    // that another transaction is about to replace, and the log entry
    // would then carry a stale `from`, a transition the history never
    // made. The lock serializes the read-modify-write on the head row.
    let row = t
        .query_opt(
            "SELECT id, payload::text, (payload IS DISTINCT FROM $4::text::jsonb)
             FROM nodes WHERE graph_id = $1 AND natural_key = $2 AND kind = $3
             FOR UPDATE",
            &[&g, &r.key, &r.kind, &payload],
        )
        .await
        .map_err(db)?;
    let Some(row) = row else {
        return Err(VerbError::NotFound(format!(
            "no {} named {:?} in graph {graph_name:?}",
            r.kind, r.key
        )));
    };
    let id: i64 = row.get(0);
    let changed: bool = row.get(2);
    if changed {
        let from: Value = serde_json::from_str(&row.get::<_, String>(1)).unwrap_or(Value::Null);
        t.execute(
            "UPDATE nodes SET payload = $2::text::jsonb, ingested_at = now() WHERE id = $1",
            &[&id, &payload],
        )
        .await
        .map_err(db)?;
        append_log(
            &t,
            id,
            &json!({"op": "update", "from": from, "to": r.payload}),
        )
        .await?;
    }
    mark_ok(&t, attempt).await?;
    t.commit().await.map_err(db)?;
    Ok(json!({
        "graph": graph_name,
        "kind": r.kind,
        "key": r.key,
        "changed": changed,
    }))
}

/// `delete`: one node and its cascade closure, measured then removed in
/// one snapshot (FR3 as amended: the store PRD's inherited cascade takes
/// the log with the node, and the audit row is the durable record).
pub async fn delete(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &KindKey,
) -> Result<Value, VerbError> {
    let graph_name = session_graph(graph)?;
    check_kind(&r.kind)?;
    let t = client.transaction().await.map_err(db)?;
    let g = graph_id(&t, graph_name).await?;
    let row = t
        .query_opt(
            "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2 AND kind = $3",
            &[&g, &r.key, &r.kind],
        )
        .await
        .map_err(db)?;
    let Some(row) = row else {
        return Err(VerbError::NotFound(format!(
            "no {} named {:?} in graph {graph_name:?}",
            r.kind, r.key
        )));
    };
    let id: i64 = row.get(0);
    let counts = t
        .query_one(
            "WITH measured AS (
               SELECT
                 (SELECT count(*) FROM edges e
                    WHERE e.src_id = $1 OR e.dst_id = $1) AS edges,
                 (SELECT count(*) FROM chunks c WHERE c.node_id = $1) AS chunks,
                 (SELECT count(*) FROM embeddings em JOIN chunks c ON c.id = em.chunk_id
                    WHERE c.node_id = $1) AS embeddings,
                 (SELECT count(*) FROM node_log l WHERE l.node_id = $1) AS log_entries
             ),
             deleted AS (DELETE FROM nodes WHERE id = $1)
             SELECT edges, chunks, embeddings, log_entries FROM measured",
            &[&id],
        )
        .await
        .map_err(db)?;
    mark_ok(&t, attempt).await?;
    t.commit().await.map_err(db)?;
    Ok(json!({
        "graph": graph_name,
        "kind": r.kind,
        "key": r.key,
        "deleted": true,
        "swept": {
            "edges": counts.get::<_, i64>(0),
            "chunks": counts.get::<_, i64>(1),
            "embeddings": counts.get::<_, i64>(2),
            "log_entries": counts.get::<_, i64>(3),
        },
    }))
}

/// `purge`: the force-gated subtree erasure (FR4): the node named by key
/// plus every node in the graph whose payload names it as parent, the
/// `list` verb's parent convention read in reverse. What ZFS snapshots
/// retain beneath Postgres is M4, named and left open.
pub async fn purge(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &PurgeRequest,
) -> Result<Value, VerbError> {
    if !r.force {
        return Err(VerbError::Denied(format!(
            "purging {:?} erases the node, its children, and their history, pass force to acknowledge",
            r.key
        )));
    }
    let graph_name = session_graph(graph)?;
    let t = client.transaction().await.map_err(db)?;
    let g = graph_id(&t, graph_name).await?;
    let root = t
        .query_opt(
            "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2",
            &[&g, &r.key],
        )
        .await
        .map_err(db)?;
    if root.is_none() {
        return Err(VerbError::NotFound(format!(
            "no node named {:?} in graph {graph_name:?}",
            r.key
        )));
    }
    let counts = t
        .query_one(
            "WITH family AS (
               SELECT id FROM nodes
                WHERE graph_id = $1
                  AND (natural_key = $2
                       OR payload->>'file_key' = $2
                       OR payload->>'doc_key' = $2)
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
            &[&g, &r.key],
        )
        .await
        .map_err(db)?;
    mark_ok(&t, attempt).await?;
    t.commit().await.map_err(db)?;
    Ok(json!({
        "graph": graph_name,
        "key": r.key,
        "purged": true,
        "swept": {
            "nodes": counts.get::<_, i64>(0),
            "edges": counts.get::<_, i64>(1),
            "chunks": counts.get::<_, i64>(2),
            "embeddings": counts.get::<_, i64>(3),
            "log_entries": counts.get::<_, i64>(4),
        },
    }))
}

async fn node_id(t: &Transaction<'_>, g: i64, key: &str) -> Result<Option<i64>, VerbError> {
    Ok(t.query_opt(
        "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2",
        &[&g, &key],
    )
    .await
    .map_err(db)?
    .map(|r| r.get(0)))
}

/// `edge.assert`: one asserted edge under R6 identity (FR6). Asserting
/// twice is a restatement: the payload is replaced and nothing errors.
pub async fn edge_assert(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &EdgeAssertRequest,
) -> Result<Value, VerbError> {
    let graph_name = session_graph(graph)?;
    check_relation(&r.relation)?;
    let payload = if r.payload.is_null() {
        json!({})
    } else {
        r.payload.clone()
    };
    let t = client.transaction().await.map_err(db)?;
    let g = graph_id(&t, graph_name).await?;
    let src = node_id(&t, g, &r.from).await?.ok_or_else(|| {
        VerbError::NotFound(format!(
            "no node named {:?} in graph {graph_name:?}",
            r.from
        ))
    })?;
    let dst = node_id(&t, g, &r.to).await?.ok_or_else(|| {
        VerbError::NotFound(format!("no node named {:?} in graph {graph_name:?}", r.to))
    })?;
    t.execute(
        "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer, payload)
         VALUES ($1, $2, $3, $4, 'asserted', 'edge.assert', $5::text::jsonb)
         ON CONFLICT (graph_id, src_id, dst_id, relation, basis)
         DO UPDATE SET payload = EXCLUDED.payload",
        &[&g, &src, &dst, &r.relation, &payload.to_string()],
    )
    .await
    .map_err(db)?;
    mark_ok(&t, attempt).await?;
    t.commit().await.map_err(db)?;
    Ok(json!({
        "graph": graph_name,
        "from": r.from,
        "to": r.to,
        "relation": r.relation,
        "basis": "asserted",
        "asserted": true,
    }))
}

/// `edge.retract`: remove one asserted edge (FR7). An edge that exists
/// with another basis is refused with the reason, because declared and
/// structural edges belong to ingest and re-ingest would restate them.
pub async fn edge_retract(
    client: &mut Client,
    graph: Option<&str>,
    attempt: Attempt,
    r: &EdgeRetractRequest,
) -> Result<Value, VerbError> {
    let graph_name = session_graph(graph)?;
    check_relation(&r.relation)?;
    let t = client.transaction().await.map_err(db)?;
    let g = graph_id(&t, graph_name).await?;
    let missing = || {
        VerbError::NotFound(format!(
            "no asserted {} edge from {:?} to {:?} in graph {graph_name:?}",
            r.relation, r.from, r.to
        ))
    };
    let Some(src) = node_id(&t, g, &r.from).await? else {
        return Err(missing());
    };
    let Some(dst) = node_id(&t, g, &r.to).await? else {
        return Err(missing());
    };
    let n = t
        .execute(
            "DELETE FROM edges
             WHERE graph_id = $1 AND src_id = $2 AND dst_id = $3
               AND relation = $4 AND basis = 'asserted'",
            &[&g, &src, &dst, &r.relation],
        )
        .await
        .map_err(db)?;
    if n == 0 {
        let other: Vec<String> = t
            .query(
                "SELECT basis::text FROM edges
                 WHERE graph_id = $1 AND src_id = $2 AND dst_id = $3 AND relation = $4",
                &[&g, &src, &dst, &r.relation],
            )
            .await
            .map_err(db)?
            .iter()
            .map(|row| row.get(0))
            .collect();
        if other.is_empty() {
            return Err(missing());
        }
        return Err(VerbError::NotFound(format!(
            "the {} edge from {:?} to {:?} has basis {}, which retract does not touch: \
             declared and structural edges belong to ingest",
            r.relation,
            r.from,
            r.to,
            other.join(", ")
        )));
    }
    mark_ok(&t, attempt).await?;
    t.commit().await.map_err(db)?;
    Ok(json!({
        "graph": graph_name,
        "from": r.from,
        "to": r.to,
        "relation": r.relation,
        "retracted": true,
    }))
}

/// FR5's crash-shaped test: run every statement the insert transaction
/// runs, then drop the transaction where a crash would kill the process,
/// and observe that nothing persisted except the attempt row with its
/// NULL outcome. Lives in-module because `insert_txn` is the product
/// code under test and stays crate-private.
#[cfg(test)]
mod tests {
    use super::*;

    /// The verb layer's copy of the relation shape, pinned on the same
    /// sample the store's schema-agreement test runs against the
    /// partition CHECK itself. Both lists must move together.
    #[test]
    fn the_relation_shape_matches_the_open_partitions() {
        for good in ["asserts", "floor-link", "depends_on", "a", "conforms"] {
            assert!(check_relation(good).is_ok(), "{good:?}");
        }
        for bad in ["Asserts", "has space", "9lives", "-leading", ""] {
            assert!(check_relation(bad).is_err(), "{bad:?}");
        }
        assert!(check_relation(&"a".repeat(63)).is_ok());
        assert!(check_relation(&"a".repeat(64)).is_err());
    }

    const PORT: u16 = 5433;

    fn socket_dir() -> Option<String> {
        let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.local/share/yeomna/run")
        });
        let sock = format!("{dir}/.s.PGSQL.{PORT}");
        std::path::Path::new(&sock).exists().then_some(dir)
    }

    #[tokio::test]
    async fn a_crash_before_commit_leaves_no_trace_but_the_attempt() {
        let Some(dir) = socket_dir() else {
            eprintln!("SKIP: no cluster socket");
            return;
        };
        let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
            eprintln!("SKIP: cannot connect as yeomna_owner");
            return;
        };
        yeomna_store::apply_schema(&owner).await.expect("schema");
        owner
            .execute(
                "INSERT INTO graphs (name) VALUES ('crash_graph') ON CONFLICT (name) DO NOTHING",
                &[],
            )
            .await
            .unwrap();
        let Ok(mut app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
            eprintln!("SKIP: yeomna_app is not provisioned");
            return;
        };
        owner
            .execute("DELETE FROM audit_log WHERE actor = 'crash-shape'", &[])
            .await
            .unwrap();
        owner
            .execute(
                "DELETE FROM nodes WHERE natural_key = 'crash_node'
                   AND graph_id = (SELECT id FROM graphs WHERE name = 'crash_graph')",
                &[],
            )
            .await
            .unwrap();

        // The attempt commits first, exactly as call() does.
        let attempt = crate::audit::begin(
            &app,
            "crash-shape",
            "insert",
            &json!({"kind": "document", "key": "crash_node"}),
        )
        .await
        .expect("the attempt row commits before the verb runs");

        // Every product statement runs, and then the transaction drops
        // where a crash would kill the process before COMMIT.
        {
            let t = app.transaction().await.unwrap();
            let out = insert_txn(
                &t,
                "crash_graph",
                attempt,
                &WriteRequest {
                    kind: "document".into(),
                    key: "crash_node".into(),
                    payload: json!({"present": true}),
                },
            )
            .await
            .expect("the statements themselves succeed");
            assert_eq!(out["created"], true, "the mutation ran inside the txn");
            drop(t);
        }

        // Nothing persisted: no head, no log, and the attempt's outcome
        // is NULL, which is the crash story the schema promises.
        let head = owner
            .query_opt("SELECT 1 FROM nodes WHERE natural_key = 'crash_node'", &[])
            .await
            .unwrap();
        assert!(head.is_none(), "the head row must roll back");
        let log: i64 = owner
            .query_one(
                "SELECT count(*) FROM node_log l JOIN nodes n ON n.id = l.node_id
                 WHERE n.natural_key = 'crash_node'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(log, 0, "the log entry must roll back");
        let outcome: Option<String> = owner
            .query_one(
                "SELECT outcome FROM audit_log WHERE id = $1",
                &[&attempt.id()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            outcome.is_none(),
            "the attempt stays NULL, telling the crash story, got {outcome:?}"
        );
    }
}
