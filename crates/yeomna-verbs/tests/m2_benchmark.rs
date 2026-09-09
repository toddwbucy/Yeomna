//! The M2 benchmark (spec 016): recursive CTEs at depth on the real code
//! graph, both formulations, report-only (R21 D8).
//!
//! Runs only when asked. Every statement is a read. The numbers go into
//! `docs/measurements/M2-recursive-cte-at-depth.md`, and nothing here
//! asserts a threshold: the charter asked for a measurement.

use std::time::Instant;

use tokio_postgres::Client;
use yeomna_verbs::*;

const PORT: u16 = 5433;
/// The dogfood graph by default. `YEOMNA_M2_GRAPH` names another real
/// graph in the same database, which is how the WeaverTools census graph
/// became the second data point.
fn graph_name() -> String {
    std::env::var("YEOMNA_M2_GRAPH").unwrap_or_else(|_| "yeomna_self".to_string())
}
/// The depths the D7 walk is driven to. 100 is the verb's own clamp.
const D7_DEPTHS: [u32; 9] = [1, 2, 3, 5, 10, 20, 30, 50, 100];
/// High enough that the cap is not what stops the walk on this graph.
const D7_CAP: u32 = 1_000_000;
/// The reference shape's depths, stopped early by its bounds.
const REF_DEPTHS: [i32; 8] = [1, 2, 3, 5, 10, 15, 20, 30];
const REF_ROW_LIMIT: i64 = 5_000_000;
const REF_TIMEOUT: &str = "60s";

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

/// Temp bytes written by this database, the spill signal. The counter is
/// cumulative and database-wide, so it is read as a difference across one
/// query and it would count a concurrent session's spill as this query's.
/// The benchmark runs against a dev cluster with one caller, and the flush
/// is forced first because the statistics are buffered per backend and an
/// unflushed read would report zero spill for a query that spilled.
async fn temp_bytes(owner: &Client) -> i64 {
    owner
        .execute("SELECT pg_stat_force_next_flush()", &[])
        .await
        .unwrap();
    owner
        .query_one(
            "SELECT temp_bytes FROM pg_stat_database WHERE datname = current_database()",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

async fn graph_id(owner: &Client, graph: &str) -> Option<i64> {
    owner
        .query_opt("SELECT id FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap()
        .map(|r| r.get(0))
}

/// The hubs: the three symbols with the most outgoing `calls` edges,
/// which is where a walk has the most room to grow.
async fn hubs(owner: &Client, g: i64) -> Vec<(String, i64)> {
    owner
        .query(
            "SELECT n.natural_key, count(*) FROM edges e JOIN nodes n ON n.id = e.src_id
             WHERE e.graph_id = $1 AND e.relation = 'calls'
             GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 3",
            &[&g],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

/// The reference's shape: UNION ALL, a path array, the cycle guard. The
/// store PRD identified this as the formulation that explodes, and the
/// benchmark must show it on the real graph under bounds.
const REFERENCE_SQL: &str = "WITH RECURSIVE walk(node, depth, path) AS (
        SELECT $1::bigint, 0, ARRAY[$1::bigint]
        UNION ALL
        SELECT e.dst_id, w.depth + 1, w.path || e.dst_id
        FROM walk w JOIN edges e ON e.src_id = w.node AND e.graph_id = $2
        WHERE w.depth < $3 AND e.dst_id <> ALL(w.path)
          AND (cardinality($4::text[]) = 0 OR e.relation = ANY($4))
    )
    SELECT count(*) FROM (SELECT 1 FROM walk LIMIT $5) t";

#[tokio::test]
#[ignore]
async fn m2_recursive_ctes_at_depth_on_the_real_graph() {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_owner");
        return;
    };
    let graph = graph_name();
    let Some(g) = graph_id(&owner, &graph).await else {
        eprintln!("SKIP: no {graph} graph, run the dogfood ingest first");
        return;
    };
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app connects");
    let session = Session::new(app, "m2-benchmark").with_graph(graph.clone());

    // -- Corpus facts ------------------------------------------------------
    let facts = owner
        .query_one(
            "SELECT
               (SELECT count(*) FROM nodes WHERE graph_id = $1),
               (SELECT count(*) FROM edges WHERE graph_id = $1),
               (SELECT count(*) FROM edges WHERE graph_id = $1 AND relation = 'calls'),
               (SELECT count(*) FROM edges a JOIN edges b
                  ON a.dst_id = b.src_id AND b.dst_id = a.src_id AND a.src_id < b.src_id
                WHERE a.graph_id = $1 AND b.graph_id = $1
                  AND a.relation = 'calls' AND b.relation = 'calls')",
            &[&g],
        )
        .await
        .unwrap();
    println!("== corpus: {graph} ==");
    println!(
        "nodes {} edges {} calls {} two-cycles-on-calls {}",
        facts.get::<_, i64>(0),
        facts.get::<_, i64>(1),
        facts.get::<_, i64>(2),
        facts.get::<_, i64>(3)
    );
    let hubs = hubs(&owner, g).await;
    if hubs.is_empty() {
        // Not a failure: a graph with no calls edges has nothing for a
        // walk to grow through, which is the finding for that corpus.
        println!("== no calls edges in {graph}, nothing to walk ==");
        return;
    }
    println!("== hubs by calls out-degree ==");
    for (k, d) in &hubs {
        println!("  {k} ({d})");
    }

    // Cycle evidence per hub: does the walk come back to the hub at a
    // depth above zero. UNION dedups on (node, depth), so a revisit at a
    // deeper depth is a distinct row and shows up.
    println!("== cycle evidence: hub reached again at depth > 0 (calls, depth 20) ==");
    for (k, _) in &hubs {
        let back: Option<i32> = owner
            .query_one(
                "WITH RECURSIVE walk(node, depth) AS (
                     SELECT n.id, 0 FROM nodes n WHERE n.graph_id = $1 AND n.natural_key = $2
                     UNION
                     SELECT e.dst_id, w.depth + 1 FROM walk w
                     JOIN edges e ON e.src_id = w.node AND e.graph_id = $1
                     WHERE w.depth < 20 AND e.relation = 'calls'
                 )
                 SELECT min(depth)::int FROM walk
                 WHERE depth > 0 AND node = (SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2)",
                &[&g, k],
            )
            .await
            .unwrap()
            .get(0);
        println!(
            "  {k}: {}",
            back.map_or("no return".to_string(), |d| format!("returns at depth {d}"))
        );
    }

    // -- D7, the shipped walk, through the verb ----------------------------
    for (label, relations) in [
        ("calls", vec!["calls".to_string()]),
        ("all relations", vec![]),
    ] {
        println!("== D7 walk, {label}: start | depth | visited | capped | ms | spill bytes ==");
        for (k, _) in &hubs {
            for depth in D7_DEPTHS {
                let req = Verb::GraphTraverse(TraverseRequest {
                    graph: graph.clone(),
                    start: k.clone(),
                    relations: relations.clone(),
                    bases: vec![],
                    depth,
                    limit: D7_CAP,
                });
                // Once to warm, once to measure, so the number is the
                // query and not the cache filling.
                let warm = session.call(&req).await;
                assert!(warm.success, "traverse failed: {:?}", warm.error);
                let before = temp_bytes(&owner).await;
                let t = Instant::now();
                let env = session.call(&req).await;
                let ms = t.elapsed().as_millis();
                let spill = temp_bytes(&owner).await - before;
                let d = env.data.expect("traverse answers");
                println!(
                    "  {k} | {depth} | {} | {} | {ms} | {spill}",
                    d["nodes"].as_array().map_or(0, Vec::len),
                    d["truncated"]
                );
            }
        }
    }

    // -- The reference's shape, bounded -------------------------------------
    owner
        .batch_execute(&format!("SET statement_timeout = '{REF_TIMEOUT}'"))
        .await
        .unwrap();
    for (label, relations) in [
        ("calls", vec!["calls".to_string()]),
        ("all relations", vec![]),
    ] {
        println!(
            "== reference shape (UNION ALL + path), {label}, limit {REF_ROW_LIMIT} rows, timeout {REF_TIMEOUT}: start | depth | rows | ms | spill bytes | stopped by =="
        );
        for (k, _) in &hubs {
            let start: i64 = owner
                .query_one(
                    "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2",
                    &[&g, k],
                )
                .await
                .unwrap()
                .get(0);
            for depth in REF_DEPTHS {
                // Warmed the same way the D7 side is, so the two columns
                // of timings mean the same thing.
                let _ = owner
                    .query_one(
                        REFERENCE_SQL,
                        &[&start, &g, &depth, &relations, &REF_ROW_LIMIT],
                    )
                    .await;
                let before = temp_bytes(&owner).await;
                let t = Instant::now();
                let result = owner
                    .query_one(
                        REFERENCE_SQL,
                        &[&start, &g, &depth, &relations, &REF_ROW_LIMIT],
                    )
                    .await;
                let ms = t.elapsed().as_millis();
                let spill = temp_bytes(&owner).await - before;
                match result {
                    Ok(row) => {
                        let rows: i64 = row.get(0);
                        let stopped = if rows >= REF_ROW_LIMIT {
                            "row limit"
                        } else {
                            "exhausted"
                        };
                        println!("  {k} | {depth} | {rows} | {ms} | {spill} | {stopped}");
                        if rows >= REF_ROW_LIMIT {
                            break;
                        }
                    }
                    Err(e) => {
                        let code = e
                            .as_db_error()
                            .map(|d| d.code().code().to_string())
                            .unwrap_or_default();
                        println!("  {k} | {depth} | - | {ms} | {spill} | error {code} ({e})");
                        break;
                    }
                }
            }
        }
    }
    owner
        .batch_execute("RESET statement_timeout")
        .await
        .unwrap();
}
