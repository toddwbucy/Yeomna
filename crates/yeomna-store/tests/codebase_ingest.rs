//! The codebase orchestrator against the live store, per spec 011.
//!
//! Lives here rather than in `yeomna-pipeline` because the orchestrator is
//! generic over its sink and the only sink is this crate's, which depends
//! on the pipeline. The test follows the dependency, not the code.
//!
//! Same gate as the other cluster tests: skip without a socket, skip
//! without the runtime role provisioned.

use serde_json::Value;
use tempfile::TempDir;
use tokio_postgres::Client;
use yeomna_chunking::TokenChunking;
use yeomna_pipeline::{CodebaseConfig, CodebaseSummary, ingest_codebase};
use yeomna_store::{PgSink, apply_schema, connect};

const PORT: u16 = 5433;

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

async fn fixtures(graph: &str) -> Option<(Client, PgSink)> {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
        return None;
    };
    let Ok(owner) = connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
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
    apply_schema(&owner).await.expect("schema applies");
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap();
    let app = connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    let sink = PgSink::new(app, graph).await.expect("graph resolves");
    Some((owner, sink))
}

/// A two-file crate where one file calls into the other, so the corpus has
/// a cross-file edge to resolve and not only per-file stars.
fn scratch_tree() -> TempDir {
    let d = TempDir::new().unwrap();
    std::fs::write(
        d.path().join("helper.rs"),
        r#"
/// A helper that the entry point calls.
pub fn compute_total(values: &[i64]) -> i64 {
    values.iter().sum()
}

pub struct Config {
    pub name: String,
}
"#,
    )
    .unwrap();
    std::fs::write(
        d.path().join("main.rs"),
        r#"
mod helper;

pub fn run() -> i64 {
    let values = vec![1, 2, 3];
    compute_total(&values)
}
"#,
    )
    .unwrap();
    d
}

async fn ingest(sink: &PgSink, root: &std::path::Path) -> CodebaseSummary {
    ingest_codebase(
        root,
        sink,
        &TokenChunking::default(),
        None,
        &CodebaseConfig::default(),
    )
    .await
    .expect("ingest succeeds")
}

async fn count(c: &Client, sql: &str, graph: &str) -> i64 {
    c.query_one(sql, &[&graph]).await.unwrap().get(0)
}

#[tokio::test]
async fn a_tree_ingests_into_nodes_edges_and_chunks() {
    let Some((owner, sink)) = fixtures("ingest_basic").await else {
        return;
    };
    let tree = scratch_tree();
    let s = ingest(&sink, tree.path()).await;

    assert_eq!(s.files_seen, 2, "both source files were offered");
    assert_eq!(s.files_written, 2);
    assert_eq!(s.files_failed, 0);
    assert!(s.symbols_written >= 3, "symbols: {}", s.symbols_written);
    assert!(s.chunks_written >= 2, "chunks: {}", s.chunks_written);
    assert!(s.edges_written >= 3, "edges: {}", s.edges_written);

    // Nodes carry their kinds and their ingest time (R9).
    let kinds: Vec<String> = owner
        .query(
            "SELECT DISTINCT n.kind FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ingest_basic' ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert!(kinds.contains(&"file".to_string()), "kinds: {kinds:?}");
    assert!(kinds.contains(&"callable".to_string()), "kinds: {kinds:?}");
    let unstamped = count(
        &owner,
        "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
         WHERE g.name = $1 AND n.ingested_at IS NULL",
        "ingest_basic",
    )
    .await;
    assert_eq!(unstamped, 0, "R9: every node is stamped");
}

#[tokio::test]
async fn every_edge_carries_provenance_and_defines_is_declared() {
    let Some((owner, sink)) = fixtures("ingest_edges").await else {
        return;
    };
    let tree = scratch_tree();
    ingest(&sink, tree.path()).await;

    // Q1 in practice: the schema forbids a null analyzer, so this asserts
    // the orchestrator supplies a real one rather than a placeholder.
    let blank = count(
        &owner,
        "SELECT count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
         WHERE g.name = $1 AND (e.analyzer = '' OR e.analyzer = 'unknown')",
        "ingest_edges",
    )
    .await;
    assert_eq!(blank, 0, "no edge is attributed to nobody");

    let rows = owner
        .query(
            "SELECT e.relation, e.basis::text, count(*)
             FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = 'ingest_edges'
             GROUP BY 1, 2 ORDER BY 1",
            &[],
        )
        .await
        .unwrap();
    let pairs: Vec<(String, String, i64)> = rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
    assert!(
        pairs
            .iter()
            .any(|(rel, basis, _)| rel == "defines" && basis == "declared"),
        "Phase 4: a file declaring a symbol is `declared`: {pairs:?}"
    );
    assert!(
        pairs.iter().all(|(_, basis, _)| basis != "asserted"),
        "nothing this orchestrator writes is inferred: {pairs:?}"
    );
}

#[tokio::test]
async fn chunks_name_the_symbols_they_cover() {
    let Some((owner, sink)) = fixtures("ingest_symlink").await else {
        return;
    };
    let tree = scratch_tree();
    ingest(&sink, tree.path()).await;

    let linked = count(
        &owner,
        "SELECT count(*) FROM chunks c
         JOIN nodes n ON n.id = c.node_id JOIN graphs g ON g.id = n.graph_id
         WHERE g.name = $1 AND cardinality(c.symbol_ids) > 0",
        "ingest_symlink",
    )
    .await;
    assert!(linked > 0, "FR 4: some chunk links to its symbols");
}

#[tokio::test]
async fn a_second_ingest_skips_everything_and_writes_no_history() {
    let Some((owner, sink)) = fixtures("ingest_idem").await else {
        return;
    };
    let tree = scratch_tree();
    let first = ingest(&sink, tree.path()).await;
    assert_eq!(first.files_skipped, 0, "nothing to skip on a cold graph");

    let log_after_first = count(
        &owner,
        "SELECT count(*) FROM node_log l JOIN nodes n ON n.id = l.node_id
         JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1",
        "ingest_idem",
    )
    .await;
    assert!(log_after_first > 0, "R8: the first write is history");

    let nodes_after_first = count(
        &owner,
        "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
         WHERE g.name = $1",
        "ingest_idem",
    )
    .await;

    let second = ingest(&sink, tree.path()).await;
    assert_eq!(second.files_skipped, 2, "FR 1: both files were unchanged");
    assert_eq!(second.files_written, 0);
    assert_eq!(
        second.edges_written, 0,
        "a warm run does not re-resolve edges either"
    );

    let nodes_after_second = count(
        &owner,
        "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
         WHERE g.name = $1",
        "ingest_idem",
    )
    .await;
    assert_eq!(nodes_after_first, nodes_after_second, "no new rows");

    let log_after_second = count(
        &owner,
        "SELECT count(*) FROM node_log l JOIN nodes n ON n.id = l.node_id
         JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1",
        "ingest_idem",
    )
    .await;
    assert_eq!(
        log_after_first, log_after_second,
        "R8: an unchanged re-ingest costs the history nothing"
    );
}

#[tokio::test]
async fn a_changed_file_appends_one_update_to_its_history() {
    let Some((owner, sink)) = fixtures("ingest_history").await else {
        return;
    };
    let tree = scratch_tree();
    ingest(&sink, tree.path()).await;

    // Add a symbol, which changes the file's symbol_hash.
    std::fs::write(
        tree.path().join("helper.rs"),
        r#"
pub fn compute_total(values: &[i64]) -> i64 {
    values.iter().sum()
}

pub struct Config {
    pub name: String,
}

pub fn added_later() -> bool {
    true
}
"#,
    )
    .unwrap();
    let s = ingest(&sink, tree.path()).await;
    assert_eq!(s.files_written, 1, "only the edited file is rewritten");
    assert_eq!(s.files_skipped, 1);

    let ops: Vec<String> = owner
        .query(
            "SELECT l.diff->>'op' FROM node_log l
             JOIN nodes n ON n.id = l.node_id JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ingest_history' AND n.kind = 'file'
             ORDER BY l.seq",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert!(
        ops.contains(&"update".to_string()),
        "R8: a real change is logged as an update: {ops:?}"
    );

    // The log wins: replaying the last entry reproduces the head.
    let row = owner
        .query_one(
            "SELECT n.payload->>'symbol_hash', l.diff->'to'->>'symbol_hash'
             FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             JOIN node_log l ON l.node_id = n.id
             WHERE g.name = 'ingest_history' AND n.kind = 'file'
               AND l.seq = (SELECT max(seq) FROM node_log WHERE node_id = n.id)
               AND n.payload->>'symbol_hash' IS NOT NULL
             LIMIT 1",
            &[],
        )
        .await
        .unwrap();
    let head: String = row.get(0);
    let logged: String = row.get(1);
    assert_eq!(head, logged, "the head is derivable from the log");
}

#[tokio::test]
async fn an_unparseable_file_does_not_fail_the_tree() {
    let Some((_owner, sink)) = fixtures("ingest_ec1").await else {
        return;
    };
    let tree = scratch_tree();
    // Valid extension, invalid Rust, and a binary-ish file for good measure.
    std::fs::write(tree.path().join("broken.rs"), "fn ( ) ) unclosed {{{").unwrap();
    std::fs::write(tree.path().join("notes.txt"), "not source at all").unwrap();

    let s = ingest(&sink, tree.path()).await;
    assert_eq!(s.files_seen, 3, "the .txt is not a language, so not seen");
    assert!(
        s.files_written >= 2,
        "EC-1: the good files still land: {s:?}"
    );
}

#[tokio::test]
async fn edge_payloads_survive_as_jsonb() {
    let Some((owner, sink)) = fixtures("ingest_payload").await else {
        return;
    };
    let tree = scratch_tree();
    ingest(&sink, tree.path()).await;
    let payload: Value = serde_json::from_str(
        &owner
            .query_one(
                "SELECT e.payload::text FROM edges e JOIN graphs g ON g.id = e.graph_id
                 WHERE g.name = 'ingest_payload' AND e.relation = 'defines' LIMIT 1",
                &[],
            )
            .await
            .unwrap()
            .get::<_, String>(0),
    )
    .unwrap();
    assert!(
        payload.get("analysis_tier").is_some(),
        "the tier rides the edge payload: {payload}"
    );
}

/// The dogfood run: this repository into a real graph.
///
/// Ignored by default because it is an operation rather than a check, and
/// it writes a graph that outlives the test. Run it deliberately:
///
/// ```text
/// cargo test -p yeomna-store --test codebase_ingest -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "operation, not a check: writes the yeomna_self graph"]
async fn dogfood_ingest_this_repository() {
    let Some((owner, sink)) = fixtures("yeomna_self").await else {
        return;
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    println!("ingesting {}", root.display());

    let started = std::time::Instant::now();
    let first = ingest(&sink, &root).await;
    let cold = started.elapsed();
    println!("cold run in {cold:?}: {first:#?}");

    let started = std::time::Instant::now();
    let second = ingest(&sink, &root).await;
    let warm = started.elapsed();
    println!("warm run in {warm:?}: {second:#?}");
    assert_eq!(second.files_written, 0, "a warm run rewrites nothing");

    for (label, sql) in [
        (
            "nodes by kind",
            "SELECT n.kind, count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'yeomna_self' GROUP BY 1 ORDER BY 2 DESC",
        ),
        (
            "edges by relation and basis",
            "SELECT e.relation || ' / ' || e.basis::text, count(*)
             FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = 'yeomna_self' GROUP BY 1 ORDER BY 2 DESC",
        ),
        (
            "analyzers",
            "SELECT e.analyzer, count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = 'yeomna_self' GROUP BY 1 ORDER BY 2 DESC",
        ),
    ] {
        println!("\n== {label} ==");
        for row in owner.query(sql, &[]).await.unwrap() {
            println!("  {:<40} {}", row.get::<_, String>(0), row.get::<_, i64>(1));
        }
    }
}
