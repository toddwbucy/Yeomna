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
use yeomna_pipeline::{CodebaseConfig, CodebaseSummary, HashEmbedder, ingest_codebase};
use yeomna_store::{PgSink, apply_schema, connect};

/// No embedder in this test, named as a type because `None` alone leaves
/// the `Embedder` parameter unresolved. Spec 022 made these paths generic
/// so a test can pass `HashEmbedder` and embed with no GPU.
const NO_EMBEDDER: Option<&HashEmbedder> = None;

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
        NO_EMBEDDER,
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

    // The semantic pass, when the environment has a language server.
    let started = std::time::Instant::now();
    let semantic = ingest_codebase(
        &root,
        &sink,
        &TokenChunking::default(),
        NO_EMBEDDER,
        &CodebaseConfig {
            semantic_lsp: true,
            ..CodebaseConfig::default()
        },
    )
    .await
    .expect("the semantic pass degrades rather than failing");
    println!("semantic pass in {:?}: {semantic:#?}", started.elapsed());

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

/// C++ resolves calls without a language server: libclang runs in process
/// during analysis and records call sites with USRs, so `cpp_edges` reads
/// them off the symbols the structural pass already produced.
#[tokio::test]
async fn cpp_calls_resolve_on_the_structural_path() {
    let Some((owner, sink)) = fixtures("ingest_cpp").await else {
        return;
    };
    let tree = TempDir::new().unwrap();
    std::fs::write(
        tree.path().join("helper.cpp"),
        "int compute(int x) { return x * 2; }\n",
    )
    .unwrap();
    std::fs::write(
        tree.path().join("main.cpp"),
        "int compute(int x);\nint run() { return compute(21); }\n",
    )
    .unwrap();
    let s = ingest(&sink, tree.path()).await;
    assert_eq!(s.files_failed, 0, "libclang parsed both files: {s:?}");

    let rows = owner
        .query(
            "SELECT e.relation, e.basis::text, e.analyzer
             FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = 'ingest_cpp' AND e.relation = 'calls'",
            &[],
        )
        .await
        .unwrap();
    assert!(
        !rows.is_empty(),
        "a C++ call across files resolves without any server: {s:?}"
    );
    for r in &rows {
        assert_eq!(
            r.get::<_, String>(1),
            "structural",
            "a front end resolved it"
        );
        assert_eq!(r.get::<_, String>(2), "libclang");
    }
}

/// Go degrades rather than failing when gopls is absent, which is the
/// contract every language-server pass carries. On a box with gopls this
/// asserts the pass runs and resolves instead.
#[tokio::test]
async fn go_semantic_pass_degrades_without_gopls() {
    let Some((owner, sink)) = fixtures("ingest_go").await else {
        return;
    };
    let tree = TempDir::new().unwrap();
    std::fs::write(
        tree.path().join("go.mod"),
        "module example.com/probe\n\ngo 1.21\n",
    )
    .unwrap();
    std::fs::write(
        tree.path().join("helper.go"),
        "package main\n\nfunc Compute(x int) int { return x * 2 }\n",
    )
    .unwrap();
    std::fs::write(
        tree.path().join("main.go"),
        "package main\n\nfunc Run() int { return Compute(21) }\n",
    )
    .unwrap();

    let s = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        NO_EMBEDDER,
        &CodebaseConfig {
            semantic_lsp: true,
            semantic_timeout: std::time::Duration::from_secs(20),
            ..CodebaseConfig::default()
        },
    )
    .await
    .expect("a missing language server never fails the ingest");

    // The structural graph stands either way, which is the whole point of
    // degrading rather than failing.
    assert!(s.files_written >= 2, "the Go files still landed: {s:?}");
    let defines = count(
        &owner,
        "SELECT count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
         WHERE g.name = $1 AND e.relation = 'defines'",
        "ingest_go",
    )
    .await;
    assert!(defines > 0, "structural edges survive a missing server");

    let has_gopls = std::process::Command::new("gopls")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    if has_gopls {
        assert!(s.semantic_units > 0, "gopls is present, so it should index");
    } else {
        assert_eq!(
            s.semantic_units, 0,
            "no gopls on this box, so no unit was indexed and nothing failed"
        );
        eprintln!("NOTE: gopls absent, exercised the degradation path only");
    }
}

/// Ingest with vectors, end to end, on a machine with no GPU (spec 022).
///
/// This is what the double exists for. Every part of the embedding path
/// except the model runs here: the trait, the late-chunking call, the byte
/// spans deciding what a chunk is, the cohort landing on the row, and the
/// count arriving in the summary. The service-gated tests cover the model.
#[tokio::test]
async fn embedding_lands_vectors_with_their_cohort_using_the_double() {
    let Some((owner, sink)) = fixtures("ci_embed").await else {
        return;
    };
    let tree = scratch_tree();
    let config = CodebaseConfig {
        embed: true,
        embed_task: "code".to_string(),
        ..Default::default()
    };
    let embedder = HashEmbedder::new();
    let summary = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        Some(&embedder),
        &config,
    )
    .await
    .expect("ingest with the double succeeds");

    assert!(summary.chunks_written > 0, "chunks landed");
    assert_eq!(
        summary.embeddings_written, summary.chunks_written,
        "one vector per chunk, which is what late chunking produces"
    );
    assert_eq!(summary.files_over_ceiling, 0, "the double has no ceiling");

    let row = owner
        .query_one(
            "SELECT count(*),
                    -- The triple, not the model alone. Rows sharing a model
                    -- with mixed revisions or tasks are two cohorts, and
                    -- counting only the model would call them one while the
                    -- min() assertions below still passed (R26).
                    count(DISTINCT (e.model, e.model_revision, e.task)),
                    min(e.model), min(e.task),
                    min(e.model_revision), min(vector_dims(e.vec::vector))
             FROM embeddings e
             JOIN chunks c ON c.id = e.chunk_id
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ci_embed'",
            &[],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(count as usize, summary.embeddings_written);
    assert_eq!(
        row.get::<_, i64>(1),
        1,
        "one cohort in one run, counted over the whole triple"
    );
    assert_eq!(
        row.get::<_, String>(2),
        "yeomna-test/hash-embedder",
        "a store holding the double's vectors says so on every row"
    );
    assert_eq!(
        row.get::<_, String>(3),
        "code",
        "the task the caller asked for reached the row (R26)"
    );
    assert_eq!(row.get::<_, String>(4), "1");
    assert_eq!(row.get::<_, i32>(5), 2048);
}

/// The chunk boundaries come from the embedder when embedding is on, which
/// is the substantive half of late chunking: the document is encoded first
/// and the pieces are decided afterwards. Proven by the spans, since the
/// double's whitespace tokens tile differently than the local chunker's.
#[tokio::test]
async fn late_chunking_decides_the_boundaries_and_the_spans_slice_the_file() {
    let Some((owner, sink)) = fixtures("ci_late").await else {
        return;
    };
    let tree = scratch_tree();
    let embedder = HashEmbedder::new();
    let config = CodebaseConfig {
        embed: true,
        chunking: yeomna_embed::embedding::ChunkPolicy {
            size_tokens: 6,
            overlap_tokens: 2,
        },
        ..Default::default()
    };
    let summary = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        Some(&embedder),
        &config,
    )
    .await
    .expect("ingest succeeds");
    assert!(
        summary.chunks_written > 2,
        "a 6-token window over these files is several chunks, not one: {}",
        summary.chunks_written
    );

    // Every stored chunk's text must be exactly the file's bytes at the
    // span stored beside it. That is the contract the spike found a defect
    // in, and it is checked against the file on disk rather than against
    // the embedder's own claim.
    let rows = owner
        .query(
            "SELECT n.payload->>'path', c.text, c.start_char, c.end_char
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ci_late' ORDER BY c.id",
            &[],
        )
        .await
        .unwrap();
    assert!(!rows.is_empty());
    for r in &rows {
        let path: String = r.get(0);
        let text: String = r.get(1);
        let start: i32 = r.get(2);
        let end: i32 = r.get(3);
        let source = std::fs::read_to_string(tree.path().join(&path)).expect("the file is there");
        let sliced = source
            .get(start as usize..end as usize)
            .unwrap_or_else(|| panic!("{path}: {start}..{end} is not a char boundary"));
        assert_eq!(
            sliced, text,
            "{path}: the span and the stored text disagree"
        );
    }
}

/// Asking for vectors with no embedder is refused at the entry rather than
/// producing a graph that looks complete and has none (spec 011 EC-4).
#[tokio::test]
async fn embedding_without_an_embedder_is_refused_before_anything_is_written() {
    let Some((owner, sink)) = fixtures("ci_embed_none").await else {
        return;
    };
    let tree = scratch_tree();
    let config = CodebaseConfig {
        embed: true,
        ..Default::default()
    };
    let err = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        NO_EMBEDDER,
        &config,
    )
    .await
    .expect_err("no embedder, and vectors were asked for");
    assert!(err.to_string().contains("no embedder"), "{err}");

    let nodes: i64 = owner
        .query_one(
            "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ci_embed_none'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(nodes, 0, "it refused before writing, not partway through");
}

/// An empty source file does not fail the run (found by dogfooding).
///
/// The first embedding ingest over a real tree failed on the whole tree
/// because one `.rs` file was empty and the embedder refuses an empty input
/// with `invalid-request`, which propagated as fatal. Real repositories have
/// empty files: placeholder modules, generated stubs, `__init__.py`. The
/// local chunker already returns no chunks for blank text, so the embedding
/// path agrees with it now instead of turning one empty file into a failed
/// ingest.
///
/// Exercised with the double, which refuses a blank input for the same
/// reason the service does, so this holds without a GPU.
#[tokio::test]
async fn an_empty_source_file_does_not_fail_an_embedding_run() {
    let Some((owner, sink)) = fixtures("ci_embed_empty").await else {
        return;
    };
    let d = TempDir::new().unwrap();
    std::fs::write(d.path().join("real.rs"), "pub fn a() -> i32 { 1 }\n").unwrap();
    std::fs::write(d.path().join("empty.rs"), "").unwrap();
    std::fs::write(d.path().join("blank.rs"), "\n\n   \n").unwrap();

    let embedder = HashEmbedder::new();
    let config = CodebaseConfig {
        embed: true,
        ..Default::default()
    };
    let summary = ingest_codebase(
        d.path(),
        &sink,
        &TokenChunking::default(),
        Some(&embedder),
        &config,
    )
    .await
    .expect("one empty file is not a failed tree");

    assert_eq!(summary.files_seen, 3);
    assert_eq!(summary.files_written, 3, "all three files still land");
    assert_eq!(summary.files_failed, 0);
    assert!(
        summary.embeddings_written > 0,
        "the file with content was still embedded"
    );
    assert_eq!(
        summary.embeddings_written, summary.chunks_written,
        "and every chunk that exists has a vector"
    );

    // The empty files are nodes with no chunks, which is what they are
    // without embedding too. They are not failures and not absences.
    let empty_with_chunks: i64 = owner
        .query_one(
            "SELECT count(*) FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = 'ci_embed_empty'
               AND n.payload->>'path' IN ('empty.rs', 'blank.rs')
               AND EXISTS (SELECT 1 FROM chunks c WHERE c.node_id = n.id)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(empty_with_chunks, 0, "nothing to chunk, so nothing chunked");
}

/// A second ingest without `overwrite` does not abort the run.
///
/// The first cut of the chunk and embedding writes treated any
/// `InsertOutcome::errors` as a rejection and failed the run. But `errors`
/// counts every row the sink did not create, and without `overwrite` the
/// insert is `ON CONFLICT DO NOTHING`, so a row that was already there is
/// indistinguishable from one that was refused. That turned the second
/// ingest of a changed file into a failed run, and worse, a run that failed
/// partway through with earlier files already committed.
///
/// The check is now gated on `overwrite`, where the insert upserts and a
/// non-creation can only be a rejection.
#[tokio::test]
async fn a_second_ingest_without_overwrite_does_not_abort() {
    let Some((owner, sink)) = fixtures("ci_no_overwrite").await else {
        return;
    };
    let tree = scratch_tree();
    let embedder = HashEmbedder::new();
    let config = CodebaseConfig {
        overwrite: false,
        embed: true,
        ..Default::default()
    };
    let first = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        Some(&embedder),
        &config,
    )
    .await
    .expect("the first run lands");
    assert!(first.chunks_written > 0);
    assert_eq!(first.embeddings_written, first.chunks_written);

    // Change one file so it is not hash-skipped, then run again with the
    // same non-overwrite config. Every chunk row for that file is already
    // there, which is the condition that used to abort.
    std::fs::write(
        tree.path().join("helper.rs"),
        r#"
/// A helper that the entry point calls, now with another function beside it.
pub fn compute_total(values: &[i64]) -> i64 {
    values.iter().sum()
}

pub fn compute_mean(values: &[i64]) -> i64 {
    if values.is_empty() { 0 } else { compute_total(values) / values.len() as i64 }
}

pub struct Config {
    pub name: String,
}
"#,
    )
    .unwrap();

    let second = ingest_codebase(
        tree.path(),
        &sink,
        &TokenChunking::default(),
        Some(&embedder),
        &config,
    )
    .await
    .expect("a re-ingest of a changed file is not a failure");
    assert!(
        second.files_written <= first.files_written,
        "nothing new was created, which is what DO NOTHING means"
    );

    // And the graph is whole rather than half-written.
    let (nodes, chunks): (i64, i64) = {
        let row = owner
            .query_one(
                "SELECT (SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
                          WHERE g.name = 'ci_no_overwrite'),
                        (SELECT count(*) FROM chunks c JOIN nodes n ON n.id = c.node_id
                          JOIN graphs g ON g.id = n.graph_id WHERE g.name = 'ci_no_overwrite')",
                &[],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(nodes > 0 && chunks > 0, "{nodes} nodes, {chunks} chunks");
}
