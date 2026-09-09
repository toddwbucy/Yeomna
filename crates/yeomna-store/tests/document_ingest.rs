//! The document ingest and the corpus-declared graph against the live
//! store, per spec 015. Lives here for the same reason `codebase_ingest`
//! does: the operation is generic over its sink and the only sink is this
//! crate's.
//!
//! Same gate as every cluster test: skip without a socket, skip without
//! the runtime role, and each test owns its graph.

use serde_json::Value;
use tempfile::TempDir;
use tokio_postgres::Client;
use yeomna_chunking::TokenChunking;
use yeomna_keys as keys;
use yeomna_pipeline::{
    CodebaseConfig, DocumentsConfig, NativeExtractor, ingest_codebase, ingest_documents,
    link_conforms,
};
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

/// (id, kind, payload) of one node, by key.
async fn node(owner: &Client, graph: &str, key: &str) -> Option<(i64, String, Value)> {
    owner
        .query_opt(
            "SELECT n.id, n.kind, n.payload::text FROM nodes n
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = $2",
            &[&graph, &key],
        )
        .await
        .unwrap()
        .map(|r| {
            (
                r.get(0),
                r.get(1),
                serde_json::from_str(&r.get::<_, String>(2)).unwrap(),
            )
        })
}

/// (relation, basis, analyzer, src_id, dst_id) of every edge in a graph.
async fn edges(owner: &Client, graph: &str) -> Vec<(String, String, String, i64, i64)> {
    owner
        .query(
            "SELECT e.relation, e.basis::text, e.analyzer, e.src_id, e.dst_id
             FROM edges e JOIN graphs g ON g.id = e.graph_id
             WHERE g.name = $1 ORDER BY e.relation, e.src_id, e.dst_id",
            &[&graph],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
        .collect()
}

async fn log_len(owner: &Client, graph: &str, key: &str) -> i64 {
    owner
        .query_one(
            "SELECT count(*) FROM node_log l JOIN nodes n ON n.id = l.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = $2",
            &[&graph, &key],
        )
        .await
        .unwrap()
        .get(0)
}

async fn chunk_count(owner: &Client, graph: &str, key: &str) -> i64 {
    owner
        .query_one(
            "SELECT count(*) FROM chunks c JOIN nodes n ON n.id = c.node_id
             JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.natural_key = $2",
            &[&graph, &key],
        )
        .await
        .unwrap()
        .get(0)
}

const SPEC_MD: &str = r#"# The Spec

Prose that says what the crate does, long enough to chunk into something.

## The property

Denial wins over permission, and the ordering is the security property.

```graph
node: some-claim
kind: assertion
tag: perturbation

edge: asserts
from: weaver-types
to: some-claim

edge: floor-link
from: some-claim
to: elsewhere-undeclared
```

## A bad block

```graph
node: broken-claim
kind: the SPU still decides nothing about what matters
```
"#;

/// A corpus of two documents, one declaring a graph, one plain.
fn corpus() -> TempDir {
    let d = TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("docs")).unwrap();
    std::fs::write(d.path().join("docs/spec.md"), SPEC_MD).unwrap();
    std::fs::write(
        d.path().join("notes.md"),
        "Plain notes with no headings and no graph, just words.\n",
    )
    .unwrap();
    d
}

async fn run_documents(root: &std::path::Path, sink: &PgSink) -> yeomna_pipeline::DocumentsSummary {
    ingest_documents(
        root,
        sink,
        &NativeExtractor::new(),
        &TokenChunking::default(),
        &DocumentsConfig::default(),
    )
    .await
    .expect("document ingest runs")
}

#[tokio::test]
async fn documents_land_with_chunks_and_the_corpus_declared_graph() {
    const G: &str = "di_documents";
    let Some((owner, sink)) = fixtures(G).await else {
        return;
    };
    let root = corpus();
    // A later file restating the claim with a different tag: the first
    // in sorted order wins and the restatement is a count.
    std::fs::write(
        root.path().join("docs/zz-restated.md"),
        "# Restated\n\n```graph\nnode: some-claim\nkind: assertion\ntag: review\n\nedge: asserts\nfrom: weaver-types\nto: some-claim\n```\n",
    )
    .unwrap();
    let s = run_documents(root.path(), &sink).await;

    assert_eq!(s.files_seen, 3);
    assert_eq!(s.files_written, 3);
    assert_eq!(s.files_failed, 0, "{:?}", s.refusals);
    assert!(s.chunks_written >= 3, "{s:?}");
    assert_eq!(s.blocks_seen, 3);
    assert_eq!(s.duplicates, 2, "one node and one edge restated");
    assert_eq!(
        s.blocks_refused, 1,
        "the prose kind refuses its block (EC-7)"
    );
    assert!(
        s.refusals
            .iter()
            .any(|r| r.contains("docs/spec.md:") && r.contains("lowercase word")),
        "{:?}",
        s.refusals
    );
    assert_eq!(s.nodes_declared, 1);
    assert_eq!(s.placeholders, 2, "weaver-types and elsewhere-undeclared");
    assert_eq!(s.edges_declared, 2);
    assert_eq!(s.edges_rejected, 0);
    assert_eq!(s.relations["asserts"], 1);
    assert_eq!(s.relations["floor-link"], 1, "kebab carried as written");

    // The document node: title from the first heading, hash recorded,
    // chunks attached under the document's key.
    let doc_key = keys::normalize_document_key("docs/spec.md");
    let (_, kind, payload) = node(&owner, G, &doc_key).await.expect("document node");
    assert_eq!(kind, "document");
    assert_eq!(payload["title"], "The Spec");
    assert_eq!(payload["path"], "docs/spec.md");
    assert_eq!(payload["content_hash"].as_str().unwrap().len(), 64);
    assert_eq!(payload["graph_blocks"], 2);
    assert!(chunk_count(&owner, G, &doc_key).await >= 1);
    let (_, _, notes) = node(&owner, G, &keys::normalize_document_key("notes.md"))
        .await
        .unwrap();
    assert_eq!(notes["title"], "notes", "no heading, so the stem (EC-1)");

    // The declared claim: block kind and tag in payload, kind document.
    let (claim_id, kind, payload) = node(&owner, G, "some-claim").await.expect("claim node");
    assert_eq!(kind, "document");
    assert_eq!(payload["declared_kind"], "assertion");
    assert_eq!(payload["tag"], "perturbation", "spec.md's declaration won");
    assert_eq!(payload["declared_in"], doc_key);

    // The placeholder: created, flagged, never overwriting.
    let (_, kind, payload) = node(&owner, G, "elsewhere-undeclared").await.unwrap();
    assert_eq!(kind, "document");
    assert_eq!(payload["placeholder"], true);

    // The edges: relation verbatim, declared basis, graph-block analyzer.
    let all = edges(&owner, G).await;
    assert_eq!(all.len(), 2);
    let floor = all.iter().find(|e| e.0 == "floor-link").unwrap();
    assert_eq!(
        (floor.1.as_str(), floor.2.as_str()),
        ("declared", "graph-block")
    );
    assert_eq!(floor.3, claim_id, "from the claim");
}

/// FR3, R8, R9: unchanged files skip, changed files log, nothing else moves.
#[tokio::test]
async fn re_ingest_skips_unchanged_and_logs_only_what_changed() {
    const G: &str = "di_reingest";
    let Some((owner, sink)) = fixtures(G).await else {
        return;
    };
    let root = corpus();
    run_documents(root.path(), &sink).await;
    let notes_key = keys::normalize_document_key("notes.md");
    let spec_key = keys::normalize_document_key("docs/spec.md");
    assert_eq!(log_len(&owner, G, &notes_key).await, 1);

    // Between runs an edge goes missing, as an interrupted run or a
    // rejected write would leave it. The hash still matches, and the
    // next run repairs it anyway: skipped for writing, not for
    // declaration.
    owner
        .execute(
            "DELETE FROM edges e USING graphs g
             WHERE g.id = e.graph_id AND g.name = $1 AND e.relation = 'floor-link'",
            &[&G],
        )
        .await
        .unwrap();
    assert_eq!(edges(&owner, G).await.len(), 1);

    let again = run_documents(root.path(), &sink).await;
    assert_eq!(again.files_skipped, 2, "{again:?}");
    assert_eq!(again.files_written, 0);
    assert_eq!(again.blocks_seen, 2, "blocks are read on every run");
    assert_eq!(again.edges_declared, 2, "re-declared, idempotently");
    assert_eq!(edges(&owner, G).await.len(), 2, "the missing edge is back");
    assert_eq!(
        log_len(&owner, G, &notes_key).await,
        1,
        "R8: no entry for no change"
    );
    assert_eq!(
        log_len(&owner, G, "some-claim").await,
        1,
        "R8 holds for re-declared nodes too"
    );

    std::fs::write(root.path().join("notes.md"), "Different words now.\n").unwrap();
    let third = run_documents(root.path(), &sink).await;
    assert_eq!(third.files_skipped, 1);
    assert_eq!(third.files_written, 1);
    assert_eq!(
        log_len(&owner, G, &notes_key).await,
        2,
        "the change is logged"
    );
    assert_eq!(
        log_len(&owner, G, &spec_key).await,
        1,
        "the other file stayed quiet"
    );
}

/// EC-4: the walk is sorted, so the first key in that order wins and the
/// winner is the same on every run.
#[tokio::test]
async fn key_collisions_resolve_to_the_first_in_sorted_order() {
    const G: &str = "di_collide";
    let Some((owner, sink)) = fixtures(G).await else {
        return;
    };
    let d = TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("a")).unwrap();
    std::fs::write(d.path().join("a.b.md"), "# Dotted\n\nfirst\n").unwrap();
    std::fs::write(d.path().join("a/b.md"), "# Slashed\n\nsecond\n").unwrap();
    assert_eq!(
        keys::normalize_document_key("a.b.md"),
        keys::normalize_document_key("a/b.md"),
        "the fixture collides by construction"
    );

    let s = run_documents(d.path(), &sink).await;
    assert_eq!(s.collisions, 1);
    assert_eq!(s.files_written, 1);
    assert!(
        s.refusals.iter().any(|r| r.starts_with("a/b.md:")),
        "{:?}",
        s.refusals
    );
    let (_, _, payload) = node(&owner, G, &keys::normalize_document_key("a.b.md"))
        .await
        .unwrap();
    assert_eq!(
        payload["path"], "a.b.md",
        "dot sorts before slash, so it won"
    );
}

/// A code tree whose one file answers to a claim the docs declare, plus a
/// header naming a claim nobody declared.
fn repo_with_docs() -> TempDir {
    let d = TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("docs")).unwrap();
    std::fs::write(
        d.path().join("helper.rs"),
        "//! conforms: some-claim\n//! conforms: no-such-claim\n\n/// conforms: some-claim\npub fn helper() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(
        d.path().join("docs/spec.md"),
        "# Spec\n\n```graph\nnode: some-claim\nkind: assertion\ntag: review\n\nedge: draws\nfrom: some-claim\nto: helper_rs\n```\n",
    )
    .unwrap();
    d
}

/// Placeholders promote in place when the code arrives, declarations
/// that name a code node fuse onto it, and conforms headers link files
/// to claims (FR4, FR7, EC-2).
#[tokio::test]
async fn placeholders_promote_declarations_fuse_and_conforms_links() {
    const G: &str = "di_promote";
    let Some((owner, sink)) = fixtures(G).await else {
        return;
    };
    let root = repo_with_docs();
    let file_key = keys::file_key("helper.rs");
    assert_eq!(file_key, "helper_rs", "the block's endpoint is this key");

    // Documents first: the code node does not exist yet, so the edge
    // hangs on a placeholder.
    let docs = run_documents(root.path(), &sink).await;
    assert_eq!(docs.placeholders, 1);
    assert_eq!(docs.edges_declared, 1);
    let (placeholder_id, kind, payload) = node(&owner, G, &file_key).await.unwrap();
    assert_eq!(kind, "document");
    assert_eq!(payload["placeholder"], true);

    // Then the code: the same key becomes the file node, id preserved,
    // the edge still attached (promotion is the ordinary upsert).
    ingest_codebase(
        root.path(),
        &sink,
        &TokenChunking::default(),
        None,
        &CodebaseConfig::default(),
    )
    .await
    .expect("code ingest runs");
    let (file_id, kind, payload) = node(&owner, G, &file_key).await.unwrap();
    assert_eq!(file_id, placeholder_id, "promoted in place");
    assert_eq!(kind, "file");
    assert!(payload.get("placeholder").is_none(), "{payload}");
    assert_eq!(payload["path"], "helper.rs");
    let draws = edges(&owner, G)
        .await
        .into_iter()
        .find(|e| e.0 == "draws")
        .expect("the declared edge survived promotion");
    assert_eq!(draws.4, file_id);

    // A later declaration naming the code node fuses and leaves it alone.
    std::fs::write(
        root.path().join("docs/crate.md"),
        "# Crate\n\n```graph\nnode: helper_rs\nkind: crate\n```\n",
    )
    .unwrap();
    let fused = run_documents(root.path(), &sink).await;
    assert_eq!(fused.nodes_fused, 1, "{fused:?}");
    assert_eq!(
        fused.nodes_declared, 1,
        "the unchanged spec re-declares its claim, the crate declaration fused instead"
    );
    let (_, kind, _) = node(&owner, G, &file_key).await.unwrap();
    assert_eq!(kind, "file", "the code node was not overwritten");

    // The conforms pass: one header resolves to the claim, one names a
    // claim nobody declared and is reported by name (EC-2).
    let links = link_conforms(root.path(), &sink, &DocumentsConfig::default())
        .await
        .expect("conforms pass runs");
    assert_eq!(links.files_scanned, 1);
    assert_eq!(links.headers_seen, 3);
    assert_eq!(links.duplicates, 1, "the item-level restatement");
    assert_eq!(links.edges_written, 1);
    assert_eq!(links.edges_rejected, 0);
    assert_eq!(links.unresolved, vec!["no-such-claim".to_string()]);
    let conforms = edges(&owner, G)
        .await
        .into_iter()
        .find(|e| e.0 == "conforms")
        .expect("the conforms edge");
    assert_eq!(
        (conforms.1.as_str(), conforms.2.as_str()),
        ("declared", "conforms-header")
    );
    assert_eq!(conforms.3, file_id, "from the file that carries the header");
    let (claim_id, _, _) = node(&owner, G, "some-claim").await.unwrap();
    assert_eq!(conforms.4, claim_id, "to the claim it names");
}

/// Schema 1.2.0 (R19a): a fresh stamp accepts a declared relation outside
/// the old closed list, refuses one outside identifier shape, and keeps
/// the structural list closed.
#[tokio::test]
async fn the_declared_partition_speaks_the_sources_words() {
    const G: &str = "di_schema";
    let Some((owner, _sink)) = fixtures(G).await else {
        return;
    };
    let ids: Vec<i64> = owner
        .query(
            "INSERT INTO nodes (graph_id, natural_key, kind)
             SELECT id, k, 'document' FROM graphs, unnest(ARRAY['p', 'q']) AS k
             WHERE name = $1 RETURNING id",
            &[&G],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    let insert = |relation: &'static str, basis: &'static str| {
        let (a, b) = (ids[0], ids[1]);
        let owner = &owner;
        async move {
            owner
                .execute(
                    "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
                     SELECT id, $2, $3, $4, $5::text::edge_basis, 'test' FROM graphs WHERE name = $1",
                    &[&G, &a, &b, &relation, &basis],
                )
                .await
                .map_err(|e| e.as_db_error().map(|d| d.code().code().to_string()))
        }
    };
    insert("grounds", "declared")
        .await
        .expect("open vocabulary, declared");
    insert("floor-link", "declared")
        .await
        .expect("kebab, declared");
    insert("depends-on", "asserted")
        .await
        .expect("kebab, asserted");
    assert_eq!(
        insert("Grounds", "declared").await.unwrap_err(),
        Some("23514".into()),
        "shape still checked"
    );
    assert_eq!(
        insert("grounds", "structural").await.unwrap_err(),
        Some("23514".into()),
        "structural stays closed"
    );
}

/// The FR6 acceptance run: the WeaverTools corpus, code and documents
/// and links, into a scratch graph, census printed. Needs the corpus at
/// /opt/weavertools/WeaverTools and a minute of patience.
#[tokio::test]
#[ignore]
async fn weavertools_document_graph_census() {
    const G: &str = "weavertools_census";
    let root = std::path::Path::new("/opt/weavertools/WeaverTools");
    if !root.join("Cargo.toml").exists() {
        eprintln!("SKIP: no WeaverTools corpus at {}", root.display());
        return;
    }
    let Some((owner, sink)) = fixtures(G).await else {
        return;
    };
    let started = std::time::Instant::now();
    let code = ingest_codebase(
        root,
        &sink,
        &TokenChunking::default(),
        None,
        &CodebaseConfig::default(),
    )
    .await
    .expect("code ingest");
    let docs = run_documents(root, &sink).await;
    let links = link_conforms(root, &sink, &DocumentsConfig::default())
        .await
        .expect("conforms pass");

    println!("== code ==\n{code:#?}");
    println!("== documents ==");
    println!(
        "files seen {} written {} skipped {} failed {} oversized {} collisions {}",
        docs.files_seen,
        docs.files_written,
        docs.files_skipped,
        docs.files_failed,
        docs.files_oversized,
        docs.collisions
    );
    println!(
        "chunks {} blocks {} refused {} nodes declared {} fused {} placeholders {} edges {} rejected {}",
        docs.chunks_written,
        docs.blocks_seen,
        docs.blocks_refused,
        docs.nodes_declared,
        docs.nodes_fused,
        docs.placeholders,
        docs.edges_declared,
        docs.edges_rejected
    );
    println!("== relations ==");
    for (r, n) in &docs.relations {
        println!("  {r:<24} {n}");
    }
    println!("== refusals ({}) ==", docs.refusals.len());
    for r in docs.refusals.iter().take(20) {
        println!("  {r}");
    }
    println!("== conforms ==");
    println!(
        "files scanned {} headers {} malformed {} edges {} rejected {} unresolved {}",
        links.files_scanned,
        links.headers_seen,
        links.malformed,
        links.edges_written,
        links.edges_rejected,
        links.unresolved.len()
    );
    for u in links.unresolved.iter().take(20) {
        println!("  unresolved: {u}");
    }
    let total_edges = edges(&owner, G).await.len();
    println!(
        "== graph: {total_edges} edges, {:.1}s ==",
        started.elapsed().as_secs_f64()
    );

    assert!(
        docs.blocks_seen > 300,
        "the corpus declares hundreds of blocks"
    );
    assert!(
        links.edges_written > 400,
        "the corpus carries hundreds of headers"
    );
}
