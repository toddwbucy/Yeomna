//! The seven schema claims, per spec 008: everything probed by hand during
//! cluster bring-up becomes an automated check.
//!
//! Gated on a reachable cluster: `YEOMNA_TEST_DB` names the socket
//! directory, falling back to the dev cluster's default location. When
//! neither yields a socket, every test SKIPS (passes with a stderr note),
//! so the workspace gate holds on boxes with no cluster (G2).

use tokio_postgres::Client;
use yeomna_store::{apply_schema, connect};

const PORT: u16 = 5433;

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

async fn owner() -> Option<Client> {
    let dir = socket_dir()?;
    let client = connect(&dir, PORT, "yeomna_owner", "yeomna").await.ok()?;
    apply_schema(&client).await.expect("schema applies");
    Some(client)
}

macro_rules! require_cluster {
    ($c:ident) => {
        let Some($c) = owner().await else {
            eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
            return;
        };
    };
}

/// A scratch graph whose teardown is claim 4's subject matter.
async fn scratch_graph(c: &Client, name: &str) -> i64 {
    c.execute("DELETE FROM graphs WHERE name = $1", &[&name])
        .await
        .unwrap();
    c.query_one(
        "INSERT INTO graphs (name) VALUES ($1) RETURNING id",
        &[&name],
    )
    .await
    .unwrap()
    .get(0)
}

async fn node(c: &Client, g: i64, key: &str, kind: &str) -> i64 {
    c.query_one(
        "INSERT INTO nodes (graph_id, natural_key, kind) VALUES ($1, $2, $3) RETURNING id",
        &[&g, &key, &kind],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn claim_1_partition_pruning_survives_the_recursive_term() {
    require_cluster!(c);
    let g = scratch_graph(&c, "claim1").await;
    let a = node(&c, g, "a", "callable").await;
    let b = node(&c, g, "b", "callable").await;
    for basis in ["declared", "structural", "asserted"] {
        c.execute(
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
             VALUES ($1, $2, $3, 'calls', $4::text::edge_basis, 'test')",
            &[&g, &a, &b, &basis],
        )
        .await
        .unwrap();
    }
    let plan = c
        .query(
            "EXPLAIN (COSTS OFF)
             WITH RECURSIVE walk(node, depth) AS (
                 SELECT $1::bigint, 0
                 UNION
                 SELECT e.dst_id, w.depth + 1
                 FROM walk w JOIN edges e ON e.src_id = w.node AND e.graph_id = $2
                 WHERE e.basis IN ('declared', 'structural') AND w.depth < 20
             )
             SELECT count(*) FROM walk",
            &[&a, &g],
        )
        .await
        .unwrap();
    let text: String = plan
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !text.contains("edges_asserted"),
        "asserted partition must be pruned from the plan:\n{text}"
    );
    assert!(text.contains("edges_declared"), "plan:\n{text}");
    c.execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();
}

#[tokio::test]
async fn claim_2_halfvec_indexes_where_vector_refuses() {
    require_cluster!(c);
    c.batch_execute(
        "CREATE TEMP TABLE hv (v halfvec(2048));
         CREATE INDEX ON hv USING hnsw (v halfvec_cosine_ops);",
    )
    .await
    .expect("halfvec(2048) must HNSW-index");
    let err = c
        .batch_execute(
            "CREATE TEMP TABLE fv (v vector(2048));
             CREATE INDEX ON fv USING hnsw (v vector_cosine_ops);",
        )
        .await
        .expect_err("vector(2048) must refuse HNSW");
    // The refusal must come from the server, not from a broken connection.
    // No prose matching: pgvector's wording is not part of the claim.
    assert!(
        err.as_db_error().is_some(),
        "expected a server refusal: {err}"
    );
}

#[tokio::test]
async fn claim_3_unattributed_edges_are_unrepresentable() {
    require_cluster!(c);
    let g = scratch_graph(&c, "claim3").await;
    let a = node(&c, g, "a", "callable").await;
    let b = node(&c, g, "b", "callable").await;
    for (basis, analyzer, what) in [
        (None::<&str>, Some("test"), "null basis"),
        (Some("declared"), None::<&str>, "null analyzer"),
    ] {
        let err = c
            .execute(
                "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
                 VALUES ($1, $2, $3, 'calls', $4::text::edge_basis, $5)",
                &[&g, &a, &b, &basis, &analyzer],
            )
            .await
            .expect_err(what);
        let db_err = err.as_db_error().expect("db error");
        // A null basis is refused by partition routing (23514, no partition
        // for the row) before NOT NULL fires. A null analyzer reaches the
        // column constraint (23502). Both are the same fact: unrepresentable.
        assert!(
            matches!(db_err.code().code(), "23502" | "23514"),
            "{what} must be rejected, got {}: {}",
            db_err.code().code(),
            db_err.message()
        );
    }
    c.execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();
}

#[tokio::test]
async fn claim_4_the_cascade_is_complete() {
    require_cluster!(c);
    let g = scratch_graph(&c, "claim4").await;
    let n = node(&c, g, "doc", "document").await;
    let chunk_id: i64 = c
        .query_one(
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
             VALUES ($1, 0, 'hello', 0, 5) RETURNING id",
            &[&n],
        )
        .await
        .unwrap()
        .get(0);
    let vec_literal = format!("[{}]", vec!["0"; 2048].join(","));
    c.execute(
        "INSERT INTO embeddings (chunk_id, vec, model, model_hash)
         VALUES ($1, $2::text::halfvec, 'm', 'h')",
        &[&chunk_id, &vec_literal],
    )
    .await
    .unwrap();
    // Two edges so each FK endpoint is exercised on its own: a self-edge
    // and a distinct-destination edge.
    let m = node(&c, g, "dst", "document").await;
    c.execute(
        "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
         VALUES ($1, $2, $2, 'contains', 'declared', 'test'),
                ($1, $2, $3, 'contains', 'declared', 'test')",
        &[&g, &n, &m],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO node_log (node_id, seq, diff) VALUES ($1, 1, '{}')",
        &[&n],
    )
    .await
    .unwrap();

    c.execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();

    for (table, filter, id) in [
        ("nodes", "id", n),
        ("chunks", "node_id", n),
        ("node_log", "node_id", n),
        ("embeddings", "chunk_id", chunk_id),
        ("edges", "src_id", n),
        ("edges", "dst_id", m),
    ] {
        let count: i64 = c
            .query_one(
                &format!("SELECT count(*) FROM {table} WHERE {filter} = $1"),
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 0, "{table} must be empty after graph delete");
    }
}

#[tokio::test]
async fn claim_5_idempotent_upsert_on_golden_keys() {
    require_cluster!(c);
    let g = scratch_graph(&c, "claim5").await;
    // The golden symbol key, pinned in yeomna-keys.
    let key = yeomna_keys::symbol_key("src_lib_rs", "Config::new", 12);
    assert_eq!(key, "src_lib_rs__Config__new__086f8847");
    for payload in ["{\"pass\": 1}", "{\"pass\": 2}"] {
        c.execute(
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, $2, 'callable', $3::text::jsonb)
             ON CONFLICT (graph_id, natural_key)
             DO UPDATE SET payload = EXCLUDED.payload",
            &[&g, &key, &payload],
        )
        .await
        .unwrap();
    }
    let (count, pass): (i64, serde_json::Value) = {
        let row = c
            .query_one(
                "SELECT count(*) OVER (), payload::text FROM nodes
                 WHERE graph_id = $1 AND natural_key = $2",
                &[&g, &key],
            )
            .await
            .unwrap();
        (
            row.get(0),
            serde_json::from_str(&row.get::<_, String>(1)).unwrap(),
        )
    };
    assert_eq!(count, 1, "two upserts, one row");
    assert_eq!(pass["pass"], 2, "second pass wins");
    c.execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();
}

#[tokio::test]
async fn claim_6_the_logs_are_append_only_for_the_app_role() {
    require_cluster!(owner_c);
    let g = scratch_graph(&owner_c, "claim6").await;
    let n = node(&owner_c, g, "doc", "document").await;
    owner_c
        .execute(
            "INSERT INTO node_log (node_id, seq, diff) VALUES ($1, 1, '{}')",
            &[&n],
        )
        .await
        .unwrap();

    let dir = socket_dir().unwrap();
    // A missing pg_ident mapping for the second role is environmental, the
    // same class as no cluster: skip, do not fail the workspace gate.
    let Ok(app) = connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        eprintln!("SKIP: no peer mapping for yeomna_app");
        return;
    };
    app.execute(
        "INSERT INTO audit_log (actor, verb) VALUES ('test', 'claim6')",
        &[],
    )
    .await
    .expect("app appends to audit_log");
    for stmt in [
        "UPDATE node_log SET seq = 99 WHERE node_id = $1",
        "DELETE FROM node_log WHERE node_id = $1",
    ] {
        let err = app.execute(stmt, &[&n]).await.expect_err("must be denied");
        assert_eq!(
            err.as_db_error().unwrap().code().code(),
            "42501",
            "append-only means permission denied: {stmt}"
        );
    }
    let err = app
        .execute("UPDATE audit_log SET verb = 'tampered'", &[])
        .await
        .expect_err("audit history is not rewritable");
    assert_eq!(err.as_db_error().unwrap().code().code(), "42501");
    owner_c
        .execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();
}

#[tokio::test]
async fn claim_7_the_pinned_pipeline_shapes_land_losslessly() {
    require_cluster!(c);
    let g = scratch_graph(&c, "claim7").await;
    // The chunk_doc / embedding_doc shapes, as pinned by yeomna-pipeline's
    // doc_construction_shape_unchanged test: keys via yeomna-keys, byte
    // offsets, dual-key contract.
    let doc_key = "docA";
    let chunk_key = yeomna_keys::chunk_key(doc_key, 3);
    assert_eq!(chunk_key, "docA_chunk_3");
    let emb_key = yeomna_keys::embedding_key(&chunk_key);
    assert_eq!(emb_key, "docA_chunk_3_emb");

    let n = node(&c, g, doc_key, "document").await;
    let chunk_id: i64 = c
        .query_one(
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
             VALUES ($1, 3, 'hello', 0, 5) RETURNING id",
            &[&n],
        )
        .await
        .unwrap()
        .get(0);
    let vec_literal = format!("[{}]", vec!["0.5"; 2048].join(","));
    c.execute(
        "INSERT INTO embeddings (chunk_id, vec, model, model_hash)
         VALUES ($1, $2::text::halfvec, 'jinaai/jina-embeddings-v4', $3)",
        &[
            &chunk_id,
            &vec_literal,
            &yeomna_keys::model_hash("jinaai/jina-embeddings-v4"),
        ],
    )
    .await
    .unwrap();

    // Round-trip: the row states exactly what the pipeline shape stated.
    let row = c
        .query_one(
            "SELECT c.text, c.chunk_index, c.start_char, c.end_char,
                    n.natural_key, e.model
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN embeddings e ON e.chunk_id = c.id
             WHERE c.id = $1",
            &[&chunk_id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "hello");
    assert_eq!(row.get::<_, i32>(1), 3);
    assert_eq!(row.get::<_, i32>(2), 0);
    assert_eq!(row.get::<_, i32>(3), 5);
    assert_eq!(row.get::<_, String>(4), "docA");
    assert_eq!(row.get::<_, String>(5), "jinaai/jina-embeddings-v4");
    // FTS answers over the landed chunk.
    let rank: f32 = c
        .query_one(
            "SELECT ts_rank_cd(tsv, websearch_to_tsquery('english', 'hello'))
             FROM chunks WHERE id = $1",
            &[&chunk_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(rank > 0.0, "FTS must rank the landed chunk");
    c.execute("DELETE FROM graphs WHERE id = $1", &[&g])
        .await
        .unwrap();
}
