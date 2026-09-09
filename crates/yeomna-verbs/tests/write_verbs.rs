//! The mutation verbs (spec 014): FR1 through FR7 and EC-1 through EC-6.
//!
//! Every test owns its graph and its actor, because writes are exactly
//! the thing that made shared fixtures race in earlier phases.

use serde_json::json;
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

/// Owner connection, schema applied, a fresh graph of this name, and a
/// session scoped to it.
async fn fixtures(graph: &str, actor: &str) -> Option<(Client, Session)> {
    let dir = socket_dir()?;
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
    // A clean graph per test: drop what a failed earlier run left.
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap();
    owner
        .execute("INSERT INTO graphs (name) VALUES ($1)", &[&graph])
        .await
        .unwrap();
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    Some((owner, Session::new(app, actor).with_graph(graph)))
}

async fn log_ops(owner: &Client, graph: &str, key: &str) -> Vec<(i32, String)> {
    owner
        .query(
            "SELECT l.seq, l.diff->>'op' FROM node_log l
             JOIN nodes n ON n.id = l.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = $2 ORDER BY l.seq",
            &[&graph, &key],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

#[tokio::test]
async fn insert_creates_the_head_and_the_first_log_entry() {
    const G: &str = "wv_insert";
    let Some((owner, s)) = fixtures(G, "wv-insert").await else {
        return;
    };
    let env = s
        .call(&Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: "frontend_db".into(),
            payload: json!({"engine": "postgres", "port": 5432}),
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let d = env.data.unwrap();
    assert_eq!(d["created"], true);
    let payload: String = owner
        .query_one(
            "SELECT n.payload::text FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = 'frontend_db'",
            &[&G],
        )
        .await
        .unwrap()
        .get(0);
    assert!(payload.contains("postgres"), "{payload}");
    assert_eq!(
        log_ops(&owner, G, "frontend_db").await,
        vec![(1, "insert".to_string())],
        "the log starts at seq 1 with the insert op"
    );
}

#[tokio::test]
async fn insert_of_an_existing_key_is_refused_and_changes_nothing() {
    const G: &str = "wv_insert_dup";
    let Some((owner, s)) = fixtures(G, "wv-insert-dup").await else {
        return;
    };
    let w = WriteRequest {
        kind: "document".into(),
        key: "redis".into(),
        payload: json!({"a": 1}),
    };
    assert!(s.call(&Verb::Insert(w.clone())).await.success);
    let env = s.call(&Verb::Insert(w)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("invalid-args"));
    assert_eq!(
        log_ops(&owner, G, "redis").await.len(),
        1,
        "nothing appended"
    );
}

/// EC-1: a write with no session graph refuses before any SQL.
#[tokio::test]
async fn a_write_without_a_session_graph_is_refused() {
    let Some(dir) = socket_dir() else { return };
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        return;
    };
    let s = Session::new(app, "wv-no-graph");
    let env = s
        .call(&Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: "nowhere".into(),
            payload: json!({}),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("invalid-args"));
}

#[tokio::test]
async fn update_replaces_the_head_and_logs_from_and_to() {
    const G: &str = "wv_update";
    let Some((owner, s)) = fixtures(G, "wv-update").await else {
        return;
    };
    s.call(&Verb::Insert(WriteRequest {
        kind: "document".into(),
        key: "svc".into(),
        payload: json!({"state": "up"}),
    }))
    .await;
    let env = s
        .call(&Verb::Update(WriteRequest {
            kind: "document".into(),
            key: "svc".into(),
            payload: json!({"state": "down"}),
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    assert_eq!(env.data.unwrap()["changed"], true);
    let diff: String = owner
        .query_one(
            "SELECT l.diff::text FROM node_log l JOIN nodes n ON n.id = l.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = 'svc' AND l.seq = 2",
            &[&G],
        )
        .await
        .unwrap()
        .get(0);
    let diff: serde_json::Value = serde_json::from_str(&diff).unwrap();
    assert_eq!(diff["op"], "update");
    assert_eq!(diff["from"]["state"], "up");
    assert_eq!(diff["to"]["state"], "down");
}

/// EC-2: an identical payload is an audited ok that appends nothing.
#[tokio::test]
async fn update_with_an_identical_payload_touches_nothing() {
    const G: &str = "wv_noop";
    let Some((owner, s)) = fixtures(G, "wv-noop").await else {
        return;
    };
    let w = WriteRequest {
        kind: "document".into(),
        key: "same".into(),
        payload: json!({"x": 1}),
    };
    s.call(&Verb::Insert(w.clone())).await;
    let env = s.call(&Verb::Update(w)).await;
    assert!(env.success);
    assert_eq!(env.data.unwrap()["changed"], false);
    assert_eq!(
        log_ops(&owner, G, "same").await.len(),
        1,
        "the no-op appended nothing (R8)"
    );
}

#[tokio::test]
async fn update_of_an_absent_node_is_not_found() {
    const G: &str = "wv_update_absent";
    let Some((_owner, s)) = fixtures(G, "wv-update-absent").await else {
        return;
    };
    let env = s
        .call(&Verb::Update(WriteRequest {
            kind: "document".into(),
            key: "ghost".into(),
            payload: json!({}),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("not-found"));
}

/// EC-3: delete sweeps the cascade closure and reports what it removed.
#[tokio::test]
async fn delete_sweeps_edges_chunks_and_history_and_reports_counts() {
    const G: &str = "wv_delete";
    let Some((owner, s)) = fixtures(G, "wv-delete").await else {
        return;
    };
    for key in ["a", "b", "c"] {
        s.call(&Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: key.into(),
            payload: json!({"k": key}),
        }))
        .await;
    }
    // Edges in both directions around "a", plus a chunk on it.
    for (f, t) in [("a", "b"), ("c", "a")] {
        let env = s
            .call(&Verb::EdgeAssert(EdgeAssertRequest {
                from: f.into(),
                to: t.into(),
                relation: "depends_on".into(),
                payload: json!(null),
            }))
            .await;
        assert!(env.success, "{:?}", env.error);
    }
    owner
        .execute(
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
             SELECT n.id, 0, 'about a', 0, 7 FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = 'a'",
            &[&G],
        )
        .await
        .unwrap();

    let env = s
        .call(&Verb::Delete(KindKey {
            kind: "document".into(),
            key: "a".into(),
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let swept = &env.data.unwrap()["swept"];
    assert_eq!(swept["edges"], 2, "both directions counted");
    assert_eq!(swept["chunks"], 1);
    assert_eq!(swept["embeddings"], 0);
    assert_eq!(swept["log_entries"], 1);
    let env = s
        .call(&Verb::Get(KindKey {
            kind: "document".into(),
            key: "a".into(),
        }))
        .await;
    assert!(!env.success, "the node is gone");
    // The bystanders survive.
    assert!(
        s.call(&Verb::Get(KindKey {
            kind: "document".into(),
            key: "b".into(),
        }))
        .await
        .success
    );
}

/// FR4 and EC-4: purge is force-gated, erases the subtree with its
/// history, and a re-insert starts a fresh lineage at seq 1.
#[tokio::test]
async fn purge_erases_the_subtree_and_a_reinsert_starts_fresh() {
    const G: &str = "wv_purge";
    let Some((owner, s)) = fixtures(G, "wv-purge").await else {
        return;
    };
    s.call(&Verb::Insert(WriteRequest {
        kind: "file".into(),
        key: "src/main.rs".into(),
        payload: json!({"lines": 10}),
    }))
    .await;
    s.call(&Verb::Insert(WriteRequest {
        kind: "callable".into(),
        key: "src/main.rs::main".into(),
        payload: json!({"file_key": "src/main.rs"}),
    }))
    .await;

    let refused = s
        .call(&Verb::Purge(PurgeRequest {
            key: "src/main.rs".into(),
            force: false,
        }))
        .await;
    assert!(!refused.success);
    assert!(refused.error.unwrap().starts_with("denied"));

    let env = s
        .call(&Verb::Purge(PurgeRequest {
            key: "src/main.rs".into(),
            force: true,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let swept = &env.data.unwrap()["swept"];
    assert_eq!(swept["nodes"], 2, "the file and its symbol");
    assert_eq!(swept["log_entries"], 2);
    let left: i64 = owner
        .query_one(
            "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1",
            &[&G],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0, "nothing survives the purge");

    s.call(&Verb::Insert(WriteRequest {
        kind: "file".into(),
        key: "src/main.rs".into(),
        payload: json!({"lines": 12}),
    }))
    .await;
    assert_eq!(
        log_ops(&owner, G, "src/main.rs").await,
        vec![(1, "insert".to_string())],
        "purge left nothing to continue from (EC-4)"
    );
}

#[tokio::test]
async fn edge_assert_writes_asserted_basis_and_restates_without_error() {
    const G: &str = "wv_edge";
    let Some((owner, s)) = fixtures(G, "wv-edge").await else {
        return;
    };
    for key in ["web", "db"] {
        s.call(&Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: key.into(),
            payload: json!({}),
        }))
        .await;
    }
    let assert_edge = |payload: serde_json::Value| {
        Verb::EdgeAssert(EdgeAssertRequest {
            from: "web".into(),
            to: "db".into(),
            relation: "depends_on".into(),
            payload,
        })
    };
    let env = s.call(&assert_edge(json!({"since": "boot"}))).await;
    assert!(env.success, "{:?}", env.error);
    // Restatement: same identity, new payload, no error, still one row.
    let env = s.call(&assert_edge(json!({"since": "later"}))).await;
    assert!(env.success, "{:?}", env.error);
    let rows = owner
        .query(
            "SELECT e.basis::text, e.analyzer, e.payload->>'since'
             FROM edges e JOIN graphs g ON g.id = e.graph_id WHERE g.name = $1",
            &[&G],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "asserting twice is one edge");
    assert_eq!(rows[0].get::<_, String>(0), "asserted");
    assert_eq!(rows[0].get::<_, String>(1), "edge.assert");
    assert_eq!(rows[0].get::<_, String>(2), "later", "the restatement won");
}

#[tokio::test]
async fn edge_assert_validates_endpoints_and_relation_shape() {
    const G: &str = "wv_edge_bad";
    let Some((_owner, s)) = fixtures(G, "wv-edge-bad").await else {
        return;
    };
    s.call(&Verb::Insert(WriteRequest {
        kind: "document".into(),
        key: "web".into(),
        payload: json!({}),
    }))
    .await;
    let env = s
        .call(&Verb::EdgeAssert(EdgeAssertRequest {
            from: "web".into(),
            to: "ghost".into(),
            relation: "depends_on".into(),
            payload: json!(null),
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("not-found") && msg.contains("ghost"),
        "{msg}"
    );
    let env = s
        .call(&Verb::EdgeAssert(EdgeAssertRequest {
            from: "web".into(),
            to: "web".into(),
            relation: "Depends-On".into(),
            payload: json!(null),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("invalid-args"));
}

/// FR7 and EC-5: retract removes asserted edges only, and the refusal
/// teaches the caller where other bases come from.
#[tokio::test]
async fn edge_retract_touches_asserted_and_only_asserted() {
    const G: &str = "wv_retract";
    let Some((owner, s)) = fixtures(G, "wv-retract").await else {
        return;
    };
    for key in ["x", "y"] {
        s.call(&Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: key.into(),
            payload: json!({}),
        }))
        .await;
    }
    s.call(&Verb::EdgeAssert(EdgeAssertRequest {
        from: "x".into(),
        to: "y".into(),
        relation: "feeds".into(),
        payload: json!(null),
    }))
    .await;
    let retract = Verb::EdgeRetract(EdgeRetractRequest {
        from: "x".into(),
        to: "y".into(),
        relation: "feeds".into(),
    });
    let env = s.call(&retract).await;
    assert!(env.success, "{:?}", env.error);
    let env = s.call(&retract).await;
    assert!(!env.success, "retracting twice finds nothing");
    assert!(env.error.unwrap().contains("no asserted"));

    // A structural edge, planted by the owner as ingest would.
    owner
        .execute(
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
             SELECT g.id, a.id, b.id, 'calls', 'structural', 'test'
             FROM graphs g
             JOIN nodes a ON a.graph_id = g.id AND a.natural_key = 'x'
             JOIN nodes b ON b.graph_id = g.id AND b.natural_key = 'y'
             WHERE g.name = $1",
            &[&G],
        )
        .await
        .unwrap();
    let env = s
        .call(&Verb::EdgeRetract(EdgeRetractRequest {
            from: "x".into(),
            to: "y".into(),
            relation: "calls".into(),
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.contains("structural") && msg.contains("ingest"),
        "{msg}"
    );
    let still: i64 = owner
        .query_one(
            "SELECT count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id WHERE g.name = $1",
            &[&G],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 1, "the structural edge survives (EC-5)");
}

/// EC-6: two sessions writing the same node race on the log sequence.
/// The loser surfaces a retryable failure rather than corrupting order,
/// and everyone eventually lands with distinct seqs.
#[tokio::test]
async fn concurrent_updates_keep_the_log_sequence_sound() {
    const G: &str = "wv_race";
    let Some((owner, s1)) = fixtures(G, "wv-race-1").await else {
        return;
    };
    let dir = socket_dir().unwrap();
    let app2 = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .unwrap();
    let s2 = Session::new(app2, "wv-race-2").with_graph(G);
    s1.call(&Verb::Insert(WriteRequest {
        kind: "document".into(),
        key: "contended".into(),
        payload: json!({"round": -1, "by": 0}),
    }))
    .await;

    let update = |round: i64, by: i64| {
        Verb::Update(WriteRequest {
            kind: "document".into(),
            key: "contended".into(),
            payload: json!({"round": round, "by": by}),
        })
    };
    let mut changed = 1i64; // the insert's first entry
    for round in 0..5 {
        let (u1, u2) = (update(round, 1), update(round, 2));
        let (a, b) = tokio::join!(s1.call(&u1), s2.call(&u2));
        for (env, by) in [(a, 1), (b, 2)] {
            if env.success {
                if env.data.unwrap()["changed"] == true {
                    changed += 1;
                }
                continue;
            }
            // The loser retries once, as a caller would, and must land.
            let retry = if by == 1 {
                s1.call(&update(round, by)).await
            } else {
                s2.call(&update(round, by)).await
            };
            assert!(retry.success, "the retry lands: {:?}", retry.error);
            if retry.data.unwrap()["changed"] == true {
                changed += 1;
            }
        }
    }
    let entries: Vec<(i32, serde_json::Value)> = owner
        .query(
            "SELECT l.seq, l.diff::text FROM node_log l JOIN nodes n ON n.id = l.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = 'contended' ORDER BY l.seq",
            &[&G],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| {
            (
                r.get(0),
                serde_json::from_str(&r.get::<_, String>(1)).unwrap(),
            )
        })
        .collect();
    let seqs: Vec<i32> = entries.iter().map(|(s, _)| *s).collect();
    let expect: Vec<i32> = (1..=changed as i32).collect();
    assert_eq!(seqs, expect, "seqs are dense and distinct");
    // The history is a chain: every update's `from` is the previous
    // entry's `to`. Without the head-row lock a racing writer could log
    // a `from` the head never held, and density alone would not notice.
    for pair in entries.windows(2) {
        let (prev, next) = (&pair[0].1, &pair[1].1);
        assert_eq!(
            next["from"], prev["to"],
            "seq {} must continue from seq {}",
            pair[1].0, pair[0].0
        );
    }
}
