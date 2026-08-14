//! Integration tests for `PgSink`, per spec 009: the sink driven through
//! the `IngestSink` trait exactly as the orchestrator drives it, including
//! the five-call sequence and the fewer-chunks rerun.
//!
//! Same gate as the schema claims: tests SKIP without a cluster socket, and
//! the sink role (`yeomna_app`) skips separately when its peer mapping is
//! absent, so the workspace gate holds everywhere (G2).

use serde_json::{Value, json};
use tokio_postgres::Client;
use yeomna_pipeline::sink::IngestSink;
use yeomna_store::{PgSink, StoreError, apply_schema, connect};

const PORT: u16 = 5433;
const MODEL: &str = "jinaai/jina-embeddings-v4";

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

/// Owner connection for schema application and row inspection, plus a
/// sink over the runtime role for the graph named. `None` skips the test.
async fn fixtures(graph: &str) -> Option<(Client, PgSink)> {
    let dir = socket_dir()?;
    let owner = connect(&dir, PORT, "yeomna_owner", "yeomna").await.ok()?;
    apply_schema(&owner).await.expect("schema applies");
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap();
    // Provisioning is the environmental question and pg_roles answers it
    // exactly, where sniffing an auth error would also swallow a real
    // misconfiguration and skip the whole suite silently.
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
    let app = connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    let sink = PgSink::new(app, graph).await.expect("graph resolves");
    Some((owner, sink))
}

macro_rules! require_sink {
    ($owner:ident, $sink:ident, $graph:literal) => {
        let Some(($owner, $sink)) = fixtures($graph).await else {
            eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
            return;
        };
    };
}

/// The pinned metadata shape from the orchestrator's `store()`.
fn metadata_doc(doc_key: &str, chunk_count: usize) -> Value {
    json!({
        "_key": doc_key,
        "full_text": "hello world, twice over",
        "tables": 0,
        "equations": 0,
        "images": 0,
        "chunk_count": chunk_count,
        "embedding_model": MODEL,
        "embedding_dimension": 2048,
        "extractor_metadata": {"backend": "test"},
    })
}

/// The pinned chunk_doc shape (DEFAULT profile: foreign key `parent_key`).
fn chunk_doc(doc_key: &str, i: usize) -> Value {
    json!({
        "_key": yeomna_keys::chunk_key(doc_key, i),
        "doc_key": doc_key,
        "parent_key": doc_key,
        "text": format!("chunk number {i}"),
        "chunk_index": i,
        "total_chunks": 3,
        "start_char": i * 10,
        "end_char": i * 10 + 8,
    })
}

/// The pinned embedding_doc shape.
fn embedding_doc(doc_key: &str, i: usize) -> Value {
    let ck = yeomna_keys::chunk_key(doc_key, i);
    json!({
        "_key": yeomna_keys::embedding_key(&ck),
        "chunk_key": ck,
        "doc_key": doc_key,
        "parent_key": doc_key,
        "embedding": vec![0.25_f32; 2048],
    })
}

/// Replay the orchestrator's exact five-call order for one document.
async fn five_calls(sink: &PgSink, doc_key: &str, chunks: usize, overwrite: bool) {
    if overwrite {
        sink.remove_documents_by_fields("chunks", &["doc_key", "parent_key"], doc_key)
            .await
            .unwrap();
        sink.remove_documents_by_fields("embeddings", &["doc_key", "parent_key"], doc_key)
            .await
            .unwrap();
    }
    let meta = sink
        .insert_documents("documents", &[metadata_doc(doc_key, chunks)], overwrite)
        .await
        .unwrap();
    assert_eq!((meta.created, meta.errors), (1, 0), "metadata lands");
    let chunk_docs: Vec<Value> = (0..chunks).map(|i| chunk_doc(doc_key, i)).collect();
    let ch = sink
        .insert_documents("chunks", &chunk_docs, overwrite)
        .await
        .unwrap();
    assert_eq!((ch.created, ch.errors), (chunks, 0), "chunks land");
    let emb_docs: Vec<Value> = (0..chunks).map(|i| embedding_doc(doc_key, i)).collect();
    let em = sink
        .insert_documents("embeddings", &emb_docs, overwrite)
        .await
        .unwrap();
    assert_eq!((em.created, em.errors), (chunks, 0), "embeddings land");
}

async fn counts(owner: &Client, graph: &str, doc_key: &str) -> (i64, i64, i64) {
    let row = owner
        .query_one(
            "SELECT
               (SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1 AND n.natural_key = $2),
               (SELECT count(*) FROM chunks c JOIN nodes n ON n.id = c.node_id
                 JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1 AND n.natural_key = $2),
               (SELECT count(*) FROM embeddings e JOIN chunks c ON c.id = e.chunk_id
                 JOIN nodes n ON n.id = c.node_id
                 JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = $1 AND n.natural_key = $2)",
            &[&graph, &doc_key],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2))
}

#[tokio::test]
async fn five_call_sequence_and_fewer_chunks_rerun_leave_no_orphans() {
    require_sink!(owner, sink, "sink_five_calls");
    five_calls(&sink, "docA", 3, true).await;
    assert_eq!(counts(&owner, "sink_five_calls", "docA").await, (1, 3, 3));
    // The load-bearing rerun: fewer chunks, preceded by the two removal
    // calls, must leave no orphaned chunks or embeddings.
    five_calls(&sink, "docA", 2, true).await;
    assert_eq!(counts(&owner, "sink_five_calls", "docA").await, (1, 2, 2));
}

#[tokio::test]
async fn overwrite_true_reingest_is_idempotent() {
    require_sink!(owner, sink, "sink_idempotent");
    five_calls(&sink, "docB", 2, true).await;
    five_calls(&sink, "docB", 2, true).await;
    assert_eq!(counts(&owner, "sink_idempotent", "docB").await, (1, 2, 2));
}

#[tokio::test]
async fn overwrite_false_counts_duplicates_and_preserves_rows() {
    require_sink!(owner, sink, "sink_no_overwrite");
    let first = sink
        .insert_documents("documents", &[metadata_doc("docC", 1)], false)
        .await
        .unwrap();
    assert_eq!((first.created, first.errors), (1, 0));
    let mut changed = metadata_doc("docC", 1);
    changed["full_text"] = json!("rewritten");
    let second = sink
        .insert_documents("documents", &[changed], false)
        .await
        .unwrap();
    assert_eq!((second.created, second.errors), (0, 1), "duplicate counted");
    let stored: String = owner
        .query_one(
            "SELECT payload->>'full_text' FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'sink_no_overwrite' AND n.natural_key = 'docC'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, "hello world, twice over", "row unchanged");
}

#[tokio::test]
async fn per_document_failures_count_and_survivors_commit() {
    require_sink!(owner, sink, "sink_partial");
    sink.insert_documents("documents", &[metadata_doc("docD", 2)], true)
        .await
        .unwrap();
    // One good chunk, one malformed (missing text), one good.
    let mut bad = chunk_doc("docD", 1);
    bad.as_object_mut().unwrap().remove("text");
    let batch = [chunk_doc("docD", 0), bad, chunk_doc("docD", 2)];
    let out = sink.insert_documents("chunks", &batch, true).await.unwrap();
    assert_eq!((out.created, out.errors), (2, 1), "survivors commit");
    // A wrong-dimension vector dies at the database and counts the same
    // way, proving the savepoint isolates a mid-batch db rejection.
    let mut short = embedding_doc("docD", 0);
    short["embedding"] = json!(vec![0.1_f32; 3]);
    let batch = [short, embedding_doc("docD", 2)];
    let out = sink
        .insert_documents("embeddings", &batch, true)
        .await
        .unwrap();
    assert_eq!((out.created, out.errors), (1, 1), "db rejection isolated");
    assert_eq!(counts(&owner, "sink_partial", "docD").await, (1, 2, 1));
}

#[tokio::test]
async fn embedding_rejections_are_per_document() {
    require_sink!(_owner, sink, "sink_embed_errors");
    // EC-1: parent metadata lacks embedding_model.
    let mut meta = metadata_doc("docE", 1);
    meta.as_object_mut().unwrap().remove("embedding_model");
    sink.insert_documents("documents", &[meta], true)
        .await
        .unwrap();
    sink.insert_documents("chunks", &[chunk_doc("docE", 0)], true)
        .await
        .unwrap();
    let out = sink
        .insert_documents("embeddings", &[embedding_doc("docE", 0)], true)
        .await
        .unwrap();
    assert_eq!((out.created, out.errors), (0, 1), "EC-1: no model recorded");
    // EC-2: a chunk_key that does not round-trip through the pinned
    // format. Leading zeros parse but never reconstruct.
    let mut mangled = embedding_doc("docE", 0);
    mangled["chunk_key"] = json!("docE_chunk_007");
    let out = sink
        .insert_documents("embeddings", &[mangled], true)
        .await
        .unwrap();
    assert_eq!((out.created, out.errors), (0, 1), "EC-2 counts, key exists");
    // An embedding for a chunk that was never written.
    let out = sink
        .insert_documents("embeddings", &[embedding_doc("docE", 7)], true)
        .await
        .unwrap();
    assert_eq!((out.created, out.errors), (0, 1), "unknown chunk counts");
}

#[tokio::test]
async fn caller_bugs_are_errors_not_counts() {
    require_sink!(_owner, sink, "sink_caller_bugs");
    let err = sink
        .insert_documents("symbols", &[json!({"_key": "x"})], true)
        .await
        .expect_err("unknown container");
    assert!(matches!(err, StoreError::UnknownContainer(_)));
    let err = sink
        .remove_documents_by_fields("chunks", &["text"], "docF")
        .await
        .expect_err("field outside the parent-key set");
    assert!(matches!(err, StoreError::UnknownRemovalField(_)));
    let err = sink
        .remove_documents_by_fields("documents", &["doc_key"], "docF")
        .await
        .expect_err("EC-4: metadata containers are not removal targets");
    assert!(matches!(err, StoreError::UnknownContainer(_)));
    // Empty batch: Ok with zeros, no transaction.
    let out = sink.insert_documents("chunks", &[], true).await.unwrap();
    assert_eq!((out.created, out.errors), (0, 0));
    // Removal of a parent that does not exist: idempotent no-op.
    sink.remove_documents_by_fields("chunks", &["doc_key", "parent_key"], "never-ingested")
        .await
        .expect("first-run overwrite deletes nothing");
}

#[tokio::test]
async fn codebase_profile_routes_to_file_nodes() {
    require_sink!(owner, sink, "sink_codebase");
    let mut meta = metadata_doc("src_lib_rs", 1);
    meta["file_key"] = json!("src_lib_rs");
    let out = sink
        .insert_documents("codebase_files", &[meta], true)
        .await
        .unwrap();
    assert_eq!((out.created, out.errors), (1, 0));
    let kind: String = owner
        .query_one(
            "SELECT kind FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'sink_codebase' AND n.natural_key = 'src_lib_rs'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(kind, "file", "codebase metadata lands as file nodes");
}
