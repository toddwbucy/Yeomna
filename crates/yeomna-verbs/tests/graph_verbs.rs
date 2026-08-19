//! The graph verbs against the live schema, per spec 013.
//!
//! A deterministic scratch graph where assertions know the contents, and
//! the cyclic dogfood graph where the point is that D7's dedup terminates
//! on real cycles.

use serde_json::Value;
use tokio_postgres::Client;
use yeomna_verbs::*;

const PORT: u16 = 5433;

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

async fn fixtures(graph: &str) -> Option<(Client, Session)> {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
        return None;
    };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_owner");
        return None;
    };
    let provisioned: bool = owner
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'yeomna_app')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    if !provisioned {
        eprintln!("SKIP: yeomna_app is not provisioned on this cluster");
        return None;
    }
    yeomna_store::apply_schema(&owner).await.expect("schema");
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap();
    seed(&owner, graph).await;
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    Some((owner, Session::new(app, "graph-test")))
}

/// A known shape:
///
/// ```text
///   A --calls/structural--> B --calls/structural--> C
///   A --imports/declared--> C
///   C --calls/asserted----> A     (the cycle, deliberately asserted)
///   D                              (isolated)
/// ```
async fn seed(c: &Client, graph: &str) {
    let g: i64 = c
        .query_one(
            "INSERT INTO graphs (name) VALUES ($1) RETURNING id",
            &[&graph],
        )
        .await
        .unwrap()
        .get(0);
    let mut ids = std::collections::HashMap::new();
    for key in ["A", "B", "C", "D"] {
        let id: i64 = c
            .query_one(
                "INSERT INTO nodes (graph_id, natural_key, kind) VALUES ($1, $2, 'callable')
                 RETURNING id",
                &[&g, &key],
            )
            .await
            .unwrap()
            .get(0);
        ids.insert(key, id);
    }
    for (from, to, rel, basis) in [
        ("A", "B", "calls", "structural"),
        ("B", "C", "calls", "structural"),
        ("A", "C", "imports", "declared"),
        ("C", "A", "calls", "asserted"),
    ] {
        c.execute(
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
             VALUES ($1, $2, $3, $4, $5::text::edge_basis, 'test')",
            &[&g, &ids[from], &ids[to], &rel, &basis],
        )
        .await
        .unwrap();
    }
}

fn data(e: &Envelope) -> &Value {
    assert!(e.success, "expected success, got {:?}", e.error);
    e.data.as_ref().expect("success carries data")
}

fn traverse_req(graph: &str, start: &str, bases: &[&str]) -> Verb {
    Verb::GraphTraverse(TraverseRequest {
        graph: graph.into(),
        start: start.into(),
        relations: vec![],
        bases: bases.iter().map(|b| b.to_string()).collect(),
        depth: 20,
        limit: 10_000,
    })
}

#[tokio::test]
async fn traverse_walks_and_basis_filters_hold() {
    let Some((_o, s)) = fixtures("gv_walk").await else {
        return;
    };
    // All bases: A reaches B, C, and itself back through the asserted
    // cycle, each exactly once.
    let d = data(&s.call(&traverse_req("gv_walk", "A", &[])).await).clone();
    let keys: Vec<&str> = d["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["A", "B", "C"], "each node once, D unreachable: {d}");
    assert_eq!(d["truncated"], false);

    // Declared and structural only: the asserted cycle is unvisited, not
    // filtered, which is the trust boundary D3 partitions on.
    let d = data(
        &s.call(&traverse_req("gv_walk", "A", &["declared", "structural"]))
            .await,
    )
    .clone();
    let depths: Vec<(String, i64)> = d["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            (
                n["key"].as_str().unwrap().into(),
                n["depth"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        depths,
        [("A".into(), 0), ("B".into(), 1), ("C".into(), 1)],
        "C at depth 1 through the declared import, not 2 through B"
    );
}

#[tokio::test]
async fn traverse_terminates_on_the_dogfood_cycles() {
    let Some(dir) = socket_dir() else { return };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        return;
    };
    // A start with outgoing calls in the real graph, if it exists.
    let Some(row) = owner
        .query_opt(
            "SELECT n.natural_key FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             JOIN edges e ON e.src_id = n.id AND e.relation = 'calls'
             WHERE g.name = 'yeomna_self' LIMIT 1",
            &[],
        )
        .await
        .unwrap_or(None)
    else {
        eprintln!("SKIP: no yeomna_self calls corpus, run the H3 dogfood operation");
        return;
    };
    let start: String = row.get(0);
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .unwrap();
    let s = Session::new(app, "graph-test");
    let started = std::time::Instant::now();
    let d = data(
        &s.call(&Verb::GraphTraverse(TraverseRequest {
            graph: "yeomna_self".into(),
            start,
            relations: vec!["calls".into()],
            bases: vec![],
            depth: 20,
            limit: 10_000,
        }))
        .await,
    )
    .clone();
    // D7's dedup on real cycles: it terminates, promptly, with every
    // node listed once.
    assert!(started.elapsed().as_secs() < 10, "terminated promptly");
    let n = d["nodes"].as_array().unwrap().len();
    assert!(n >= 1, "reached something: {n}");
    // A set, not Vec::dedup: dedup removes only consecutive repeats,
    // and rows here are ordered by depth first, so a repeated key would
    // not be adjacent and would survive it.
    let keys: Vec<&str> = d["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["key"].as_str().unwrap())
        .collect();
    let unique: std::collections::HashSet<&&str> = keys.iter().collect();
    assert_eq!(unique.len(), keys.len(), "each node exactly once");
}

#[tokio::test]
async fn neighbors_respects_direction() {
    let Some((_o, s)) = fixtures("gv_hop").await else {
        return;
    };
    let hop = |direction: Direction| {
        Verb::GraphNeighbors(NeighborsRequest {
            graph: "gv_hop".into(),
            key: "C".into(),
            direction,
            relations: vec![],
            bases: vec![],
            limit: 20,
        })
    };
    let d = data(&s.call(&hop(Direction::In)).await).clone();
    let inbound: Vec<&str> = d["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["key"].as_str().unwrap())
        .collect();
    assert_eq!(inbound, ["A", "B"], "B calls C, A imports C: {d}");

    let d = data(&s.call(&hop(Direction::Out)).await).clone();
    let outbound: Vec<&str> = d["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["key"].as_str().unwrap())
        .collect();
    assert_eq!(outbound, ["A"], "C reaches only the asserted cycle: {d}");

    let d = data(&s.call(&hop(Direction::Both)).await).clone();
    assert_eq!(d["neighbors"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn shortest_path_finds_shortest_and_answers_absence() {
    let Some((_o, s)) = fixtures("gv_path").await else {
        return;
    };
    let path = |from: &str, to: &str, relations: Vec<String>| {
        Verb::GraphShortestPath(ShortestPathRequest {
            graph: "gv_path".into(),
            from: from.into(),
            to: to.into(),
            relations,
            bases: vec![],
            cap: 10_000,
        })
    };
    // Unrestricted: the declared import is the one-hop path.
    let d = data(&s.call(&path("A", "C", vec![])).await).clone();
    assert_eq!(d["found"], true);
    assert_eq!(d["length"], 1);
    assert_eq!(d["path"], serde_json::json!(["A", "C"]));

    // Calls only: the way goes through B.
    let d = data(&s.call(&path("A", "C", vec!["calls".into()])).await).clone();
    assert_eq!(d["length"], 2);
    assert_eq!(d["path"], serde_json::json!(["A", "B", "C"]));

    // No path is an answer, not an error (spec 013).
    let d = data(&s.call(&path("A", "D", vec![])).await).clone();
    assert_eq!(d["found"], false);
    assert_eq!(d["path"], Value::Null);
    assert_eq!(d["truncated"], false, "it exhausted the space, honestly");
}

#[tokio::test]
async fn create_drop_and_the_force_gate() {
    let Some((owner, s)) = fixtures("gv_lifecycle").await else {
        return;
    };
    owner
        .execute("DELETE FROM graphs WHERE name = 'gv_fresh'", &[])
        .await
        .unwrap();
    let d = data(
        &s.call(&Verb::GraphCreate(GraphName {
            name: "gv_fresh".into(),
        }))
        .await,
    )
    .clone();
    assert_eq!(d["created"], true);

    // EC-2: creating twice is confusion, not idempotence.
    let env = s
        .call(&Verb::GraphCreate(GraphName {
            name: "gv_fresh".into(),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("invalid-args"));

    // The force gate: denied, and nothing swept.
    let env = s
        .call(&Verb::GraphDrop(DropRequest {
            name: "gv_lifecycle".into(),
            force: false,
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("denied"));
    let still: i64 = owner
        .query_one(
            "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'gv_lifecycle'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 4, "denied means nothing was deleted");

    // With force: dropped, and the sweep is reported.
    let d = data(
        &s.call(&Verb::GraphDrop(DropRequest {
            name: "gv_lifecycle".into(),
            force: true,
        }))
        .await,
    )
    .clone();
    assert_eq!(d["swept"]["nodes"], 4);
    assert_eq!(d["swept"]["edges"], 4);
    let gone: i64 = owner
        .query_one(
            "SELECT count(*) FROM graphs WHERE name = 'gv_lifecycle'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(gone, 0);
    // Cleanup.
    owner
        .execute("DELETE FROM graphs WHERE name = 'gv_fresh'", &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn bad_inputs_are_refused_by_name() {
    let Some((_o, s)) = fixtures("gv_refuse").await else {
        return;
    };
    // FR 4: outside the closed sets is a refusal, not an empty answer.
    let env = s.call(&traverse_req("gv_refuse", "A", &["wizardry"])).await;
    assert!(!env.success);
    assert!(env.error.unwrap().contains("declared"));

    let env = s
        .call(&Verb::GraphTraverse(TraverseRequest {
            graph: "gv_refuse".into(),
            start: "A".into(),
            relations: vec!["summons".into()],
            bases: vec![],
            depth: 20,
            limit: 100,
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().contains("calls"));

    // EC-1: a missing start is NotFound, distinct from a lonely one.
    let env = s.call(&traverse_req("gv_refuse", "nope", &[])).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("not-found"));
    let d = data(&s.call(&traverse_req("gv_refuse", "D", &[])).await).clone();
    assert_eq!(
        d["nodes"].as_array().unwrap().len(),
        1,
        "D alone at depth 0"
    );

    // R14: materialize refuses, naming what it waits for.
    let env = s
        .call(&Verb::GraphMaterialize(GraphScoped {
            graph: "gv_refuse".into(),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().contains("materialization"));
}

/// The cap bounds the walk, proven rather than argued: a complete
/// directed graph on 15 nodes holds astronomically many simple paths to
/// depth 10, so if the fetch limit did not stop the recursion this test
/// could not finish. It is the regression guard for the stop-recursion
/// idiom the traversal relies on: if Postgres ever changes that
/// behavior, this hangs visibly instead of production finding out.
#[tokio::test]
async fn the_cap_bounds_the_walk_on_a_dense_graph() {
    let Some((owner, s)) = fixtures("gv_dense").await else {
        return;
    };
    owner
        .execute("DELETE FROM graphs WHERE name = 'gv_dense_k'", &[])
        .await
        .unwrap();
    let g: i64 = owner
        .query_one(
            "INSERT INTO graphs (name) VALUES ('gv_dense_k') RETURNING id",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    // K15: every node calls every other node.
    owner
        .execute(
            "INSERT INTO nodes (graph_id, natural_key, kind)
             SELECT $1, 'n' || i, 'callable' FROM generate_series(1, 15) i",
            &[&g],
        )
        .await
        .unwrap();
    owner
        .execute(
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
             SELECT $1, a.id, b.id, 'calls', 'structural', 'test'
             FROM nodes a, nodes b
             WHERE a.graph_id = $1 AND b.graph_id = $1 AND a.id <> b.id",
            &[&g],
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let env = s
        .call(&Verb::GraphShortestPath(ShortestPathRequest {
            graph: "gv_dense_k".into(),
            from: "n1".into(),
            to: "n15".into(),
            relations: vec![],
            bases: vec![],
            cap: 2_000,
        }))
        .await;
    let elapsed = started.elapsed();
    let d = data(&env).clone();
    assert!(
        elapsed.as_secs() < 10,
        "the cap bounded the walk: {elapsed:?}"
    );
    assert_eq!(d["found"], true, "a direct edge exists: {d}");
    assert_eq!(d["length"], 1, "and level order found it first");
    // Traverse over the same graph. The first version of this asserted
    // truncation at limit 500 and learned something better: UNION dedup
    // keeps the walk to (node, depth) pairs, so K15 tops out near 285
    // rows and 500 never truncates. The dedup is what makes traversal
    // polynomial on a complete graph, which is D7's whole point, so this
    // asserts both halves: a full walk sees every node untruncated, and
    // a cap below the dedup'd row count truncates honestly.
    let traverse = |limit: u32| {
        Verb::GraphTraverse(TraverseRequest {
            graph: "gv_dense_k".into(),
            start: "n1".into(),
            relations: vec![],
            bases: vec![],
            depth: 20,
            limit,
        })
    };
    let started = std::time::Instant::now();
    let d = data(&s.call(&traverse(10_000)).await).clone();
    assert!(started.elapsed().as_secs() < 10);
    assert_eq!(d["nodes"].as_array().unwrap().len(), 15, "all of K15: {d}");
    assert_eq!(d["truncated"], false, "the dedup kept the walk small");
    let d = data(&s.call(&traverse(10)).await).clone();
    assert_eq!(d["truncated"], true, "a cap below the walk truncates: {d}");
    owner
        .execute("DELETE FROM graphs WHERE name = 'gv_dense_k'", &[])
        .await
        .unwrap();
}
