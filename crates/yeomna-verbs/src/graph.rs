//! The seven graph verbs, per spec 013.
//!
//! The traversal contract is D7 applied: `UNION` node-dedup over
//! `(node, depth)`, which claim 1 proved prunes the asserted partition
//! when the basis filter allows it. Basis values are the one place a
//! caller's input reaches SQL structure here, and they are interpolated
//! as literals only after validation against the enum's closed set,
//! because a parameterized basis array reaches the planner too late for
//! the compile-time pruning that claim proves.

use serde_json::{Value, json};

use crate::error::VerbError;
use crate::execute::Session;
use crate::verb::{
    Direction, DropRequest, GraphName, NeighborsRequest, ShortestPathRequest, TraverseRequest,
};

/// The relations the schema's CHECK admits.
const RELATIONS: [&str; 5] = ["defines", "calls", "implements", "imports", "contains"];
/// The bases the enum admits.
const BASES: [&str; 3] = ["declared", "structural", "asserted"];
/// The internal depth bound on the enumerating shortest-path walk.
const PATH_DEPTH: i32 = 20;
/// The structural bound on traversal depth, whatever the caller asks.
/// The row cap stops the walk through the fetch limit, and this clamp is
/// the guarantee that does not depend on fetch semantics.
const MAX_DEPTH: i32 = 100;

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

fn check_relations(rels: &[String]) -> Result<(), VerbError> {
    for r in rels {
        if !RELATIONS.contains(&r.as_str()) {
            return Err(VerbError::InvalidArgs(format!(
                "unknown relation {r:?}, expected one of {}",
                RELATIONS.join(", ")
            )));
        }
    }
    Ok(())
}

/// Validate bases and render the literal clause the planner can prune on.
/// Empty means all bases, which is no clause at all.
pub fn basis_clause(bases: &[String]) -> Result<String, VerbError> {
    if bases.is_empty() {
        return Ok(String::new());
    }
    for b in bases {
        if !BASES.contains(&b.as_str()) {
            return Err(VerbError::InvalidArgs(format!(
                "unknown basis {b:?}, expected one of {}",
                BASES.join(", ")
            )));
        }
    }
    let list: Vec<String> = bases.iter().map(|b| format!("'{b}'")).collect();
    Ok(format!("AND e.basis IN ({})", list.join(", ")))
}

/// The traversal SQL for a given basis filter. Crate-private: the pruning
/// proof lives in this module's own tests so it can EXPLAIN exactly what
/// the verb executes without the builder becoming a second public surface
/// beside the verbs, which charter section 6 forbids.
pub(crate) fn traverse_sql(bases: &[String]) -> Result<String, VerbError> {
    let basis = basis_clause(bases)?;
    Ok(format!(
        "WITH RECURSIVE walk(node, depth) AS (
             SELECT $1::bigint, 0
             UNION
             SELECT e.dst_id, w.depth + 1
             FROM walk w
             JOIN edges e ON e.src_id = w.node AND e.graph_id = $2
             WHERE w.depth < $3
               AND (cardinality($4::text[]) = 0 OR e.relation = ANY($4))
               {basis}
         ),
         capped AS (SELECT node, depth FROM walk LIMIT $5)
         SELECT n.natural_key, n.kind, min(c.depth)::int AS depth,
                (SELECT count(*) FROM capped)::bigint AS visited
         FROM capped c JOIN nodes n ON n.id = c.node
         GROUP BY n.natural_key, n.kind
         ORDER BY depth, n.natural_key"
    ))
}

/// Resolve a graph name to its id.
async fn graph_id(s: &Session, name: &str) -> Result<i64, VerbError> {
    s.client()
        .query_opt("SELECT id FROM graphs WHERE name = $1", &[&name])
        .await
        .map_err(db)?
        .map(|r| r.get(0))
        .ok_or_else(|| VerbError::NotFound(format!("no graph named {name:?}")))
}

/// Resolve a node key inside a graph.
async fn node_id(s: &Session, g: i64, key: &str) -> Result<i64, VerbError> {
    s.client()
        .query_opt(
            "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2",
            &[&g, &key],
        )
        .await
        .map_err(db)?
        .map(|r| r.get(0))
        .ok_or_else(|| VerbError::NotFound(format!("no node {key:?} in this graph")))
}

/// `graph.traverse`: reachability, each node once at its minimum depth.
pub async fn traverse(s: &Session, r: &TraverseRequest) -> Result<Value, VerbError> {
    check_relations(&r.relations)?;
    let sql = traverse_sql(&r.bases)?;
    let g = graph_id(s, &r.graph).await?;
    let start = node_id(s, g, &r.start).await?;
    let depth = i32::try_from(r.depth).unwrap_or(MAX_DEPTH).min(MAX_DEPTH);
    let limit = i64::from(r.limit.max(1));
    let rows = s
        .client()
        .query(&sql, &[&start, &g, &depth, &r.relations, &limit])
        .await
        .map_err(db)?;
    let visited = rows.first().map(|r| r.get::<_, i64>(3)).unwrap_or(0);
    Ok(json!({
        "graph": r.graph,
        "start": r.start,
        "nodes": rows.iter().map(|row| json!({
            "key": row.get::<_, String>(0),
            "kind": row.get::<_, String>(1),
            "depth": row.get::<_, i32>(2),
        })).collect::<Vec<_>>(),
        // An honest partial answer says so (spec 013): the cap was hit,
        // so reachable nodes may be missing.
        "truncated": visited >= limit,
    }))
}

/// `graph.neighbors`: one hop, by direction.
pub async fn neighbors(s: &Session, r: &NeighborsRequest) -> Result<Value, VerbError> {
    check_relations(&r.relations)?;
    let basis = basis_clause(&r.bases)?;
    let g = graph_id(s, &r.graph).await?;
    let id = node_id(s, g, &r.key).await?;
    let out = format!(
        "SELECT n.natural_key, n.kind, e.relation, e.basis::text, 'out' AS direction
         FROM edges e JOIN nodes n ON n.id = e.dst_id
         WHERE e.graph_id = $1 AND e.src_id = $2
           AND (cardinality($3::text[]) = 0 OR e.relation = ANY($3)) {basis}"
    );
    let inward = format!(
        "SELECT n.natural_key, n.kind, e.relation, e.basis::text, 'in' AS direction
         FROM edges e JOIN nodes n ON n.id = e.src_id
         WHERE e.graph_id = $1 AND e.dst_id = $2
           AND (cardinality($3::text[]) = 0 OR e.relation = ANY($3)) {basis}"
    );
    let body = match r.direction {
        Direction::Out => out,
        Direction::In => inward,
        Direction::Both => format!("{out} UNION ALL {inward}"),
    };
    let sql = format!("SELECT * FROM ({body}) hop ORDER BY natural_key, relation LIMIT $4");
    let limit = i64::from(r.limit.max(1));
    let rows = s
        .client()
        .query(&sql, &[&g, &id, &r.relations, &limit])
        .await
        .map_err(db)?;
    Ok(json!({
        "graph": r.graph,
        "key": r.key,
        "neighbors": rows.iter().map(|row| json!({
            "key": row.get::<_, String>(0),
            "kind": row.get::<_, String>(1),
            "relation": row.get::<_, String>(2),
            "basis": row.get::<_, String>(3),
            "direction": row.get::<_, String>(4),
        })).collect::<Vec<_>>(),
    }))
}

/// `graph.shortest-path`: the enumerating exception, capped and honest
/// about truncation. Level-order recursion means the first arrival at the
/// target inside the cap is a shortest path.
pub async fn shortest_path(s: &Session, r: &ShortestPathRequest) -> Result<Value, VerbError> {
    check_relations(&r.relations)?;
    let basis = basis_clause(&r.bases)?;
    let g = graph_id(s, &r.graph).await?;
    let from = node_id(s, g, &r.from).await?;
    let to = node_id(s, g, &r.to).await?;
    let cap = i64::from(r.cap.max(1));
    let sql = format!(
        "WITH RECURSIVE walk(node, path, depth) AS (
             SELECT $1::bigint, ARRAY[$1::bigint], 0
             UNION ALL
             SELECT e.dst_id, w.path || e.dst_id, w.depth + 1
             FROM walk w
             JOIN edges e ON e.src_id = w.node AND e.graph_id = $2
             WHERE w.depth < $3
               AND w.node <> $4
               AND e.dst_id <> ALL(w.path)
               AND (cardinality($5::text[]) = 0 OR e.relation = ANY($5))
               {basis}
         ),
         capped AS (SELECT node, path, depth FROM walk LIMIT $6)
         SELECT (SELECT count(*) FROM capped)::bigint AS visited,
                best.path, best.depth
         FROM (SELECT 1) one
         LEFT JOIN LATERAL (
             SELECT (SELECT array_agg(n.natural_key ORDER BY t.ord)
                     FROM unnest(f.path) WITH ORDINALITY AS t(id, ord)
                     JOIN nodes n ON n.id = t.id) AS path,
                    f.depth
             FROM capped f WHERE f.node = $4
             ORDER BY f.depth LIMIT 1
         ) best ON true"
    );
    let row = s
        .client()
        .query_one(&sql, &[&from, &g, &PATH_DEPTH, &to, &r.relations, &cap])
        .await
        .map_err(db)?;
    let visited: i64 = row.get(0);
    let path: Option<Vec<String>> = row.get(1);
    // No path is an answer, not an error. A truncated search that found
    // nothing says so, since no path within the cap and no path at all
    // are different facts. One walk answers both questions.
    match path {
        Some(path) => Ok(json!({
            "graph": r.graph, "from": r.from, "to": r.to,
            "found": true,
            "length": row.get::<_, i32>(2),
            "path": path,
            "truncated": visited >= cap,
        })),
        None => Ok(json!({
            "graph": r.graph, "from": r.from, "to": r.to,
            "found": false,
            "path": Value::Null,
            "truncated": visited >= cap,
        })),
    }
}

/// `graph.list`: the registry with sizes.
pub async fn list(s: &Session) -> Result<Value, VerbError> {
    let rows = s
        .client()
        .query(
            "SELECT g.name, g.created_at,
                    (SELECT count(*) FROM nodes n WHERE n.graph_id = g.id),
                    (SELECT count(*) FROM edges e WHERE e.graph_id = g.id)
             FROM graphs g ORDER BY g.name",
            &[],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "graphs": rows.iter().map(|r| {
            let at: chrono::DateTime<chrono::Utc> = r.get(1);
            json!({
                "name": r.get::<_, String>(0),
                "created_at": at.to_rfc3339(),
                "nodes": r.get::<_, i64>(2),
                "edges": r.get::<_, i64>(3),
            })
        }).collect::<Vec<_>>(),
    }))
}

/// `graph.create`: a registry row. Not idempotent at the verb layer,
/// because a caller who creates twice is confused and should hear so
/// (EC-2).
pub async fn create(s: &Session, r: &GraphName) -> Result<Value, VerbError> {
    let affected = s
        .client()
        .execute(
            "INSERT INTO graphs (name) VALUES ($1) ON CONFLICT (name) DO NOTHING",
            &[&r.name],
        )
        .await
        .map_err(db)?;
    if affected == 0 {
        return Err(VerbError::InvalidArgs(format!(
            "graph {:?} already exists",
            r.name
        )));
    }
    Ok(json!({ "graph": r.name, "created": true }))
}

/// `graph.drop`: the first destructive verb. One DELETE, which is T4 made
/// mechanical through the cascade claim 4 proves, with the swept counts
/// measured first and reported.
pub async fn drop(s: &Session, r: &DropRequest) -> Result<Value, VerbError> {
    if !r.force {
        return Err(VerbError::Denied(format!(
            "dropping graph {:?} deletes everything in it, pass force to acknowledge",
            r.name
        )));
    }
    let g = graph_id(s, &r.name).await?;
    // Counts and delete in one statement: every sub-statement of a WITH
    // runs on the same snapshot, so the reported sweep is what the
    // cascade removed, not what existed moments earlier. This is the
    // caller's only record of a destructive act.
    let counts = s
        .client()
        .query_one(
            "WITH measured AS (
               SELECT
                 (SELECT count(*) FROM nodes n WHERE n.graph_id = $1) AS nodes,
                 (SELECT count(*) FROM edges e WHERE e.graph_id = $1) AS edges,
                 (SELECT count(*) FROM chunks c JOIN nodes n ON n.id = c.node_id
                    WHERE n.graph_id = $1) AS chunks,
                 (SELECT count(*) FROM embeddings em JOIN chunks c ON c.id = em.chunk_id
                    JOIN nodes n ON n.id = c.node_id WHERE n.graph_id = $1) AS embeddings
             ),
             deleted AS (DELETE FROM graphs WHERE id = $1)
             SELECT nodes, edges, chunks, embeddings FROM measured",
            &[&g],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "graph": r.name,
        "dropped": true,
        "swept": {
            "nodes": counts.get::<_, i64>(0),
            "edges": counts.get::<_, i64>(1),
            "chunks": counts.get::<_, i64>(2),
            "embeddings": counts.get::<_, i64>(3),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 013 FR 1 at the verb level: the plan for the SQL this module
    /// executes, with bases restricted to declared and structural, does
    /// not touch the asserted partition. GENERIC_PLAN plans with the
    /// placeholders unbound, and the extended protocol would demand
    /// values, so the EXPLAIN goes through simple_query. Cluster-gated
    /// like every live test.
    #[tokio::test]
    async fn the_traversal_plan_prunes_the_asserted_partition() {
        let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.local/share/yeomna/run")
        });
        if !std::path::Path::new(&format!("{dir}/.s.PGSQL.5433")).exists() {
            eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
            return;
        }
        let Ok(owner) = yeomna_store::connect(&dir, 5433, "yeomna_owner", "yeomna").await else {
            eprintln!("SKIP: cannot connect as yeomna_owner");
            return;
        };
        let sql = traverse_sql(&["declared".into(), "structural".into()]).unwrap();
        let plan: String = owner
            .simple_query(&format!("EXPLAIN (COSTS OFF, GENERIC_PLAN) {sql}"))
            .await
            .unwrap()
            .iter()
            .filter_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(String::from),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !plan.contains("edges_asserted"),
            "claim 1 at the verb level:\n{plan}"
        );
        assert!(plan.contains("edges_declared"), "plan:\n{plan}");
    }
}
