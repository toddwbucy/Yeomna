//! The ingestion verbs (spec 019): ingest, drift, and validate through
//! the verb layer, against the live store.
//!
//! Each test owns its graph and its actor. The trees are temporary and
//! small, because what is under test is the verb wrapping the
//! orchestrator rather than the orchestrator, which spec 011 and spec
//! 015 already prove on real corpora.

use serde_json::{Value, json};
use tempfile::TempDir;
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

/// A session with an endpoint, which every ingesting verb needs, and a
/// graph that already exists, which every ingesting verb requires.
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
        .expect("yeomna_app connects");
    Some((
        owner,
        Session::new(app, actor)
            .with_graph(graph)
            .with_endpoint(dir, PORT),
    ))
}

fn data(env: &Envelope) -> &Value {
    env.data.as_ref().expect("an envelope with data")
}

/// A two-file crate, one calling into the other.
fn tree() -> TempDir {
    let d = TempDir::new().unwrap();
    std::fs::write(
        d.path().join("helper.rs"),
        "pub fn compute(values: &[i64]) -> i64 {\n    values.iter().sum()\n}\n",
    )
    .unwrap();
    std::fs::write(
        d.path().join("main.rs"),
        "mod helper;\n\npub fn run() -> i64 {\n    compute(&[1, 2, 3])\n}\n",
    )
    .unwrap();
    d
}

async fn counts(owner: &Client, graph: &str) -> (i64, i64, i64) {
    let row = owner
        .query_one(
            "SELECT (SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
                       WHERE g.name = $1),
                    (SELECT count(*) FROM edges e JOIN graphs g ON g.id = e.graph_id
                       WHERE g.name = $1),
                    (SELECT count(*) FROM node_log l JOIN nodes n ON n.id = l.node_id
                       JOIN graphs g ON g.id = n.graph_id WHERE g.name = $1)",
            &[&graph],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2))
}

/// FR1 and EC-3: a tree ingests, and ingesting it again skips
/// everything and leaves the history alone.
#[tokio::test]
async fn codebase_ingest_writes_the_graph_and_then_skips_it() {
    const G: &str = "iv_ingest";
    let Some((owner, s)) = fixtures(G, "iv-ingest").await else {
        return;
    };
    let root = tree();
    let req = Verb::CodebaseIngest(IngestRequest {
        path: root.path().to_string_lossy().to_string(),
        graph: G.into(),
        overwrite: true,
    });

    let env = s.call(&req).await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["files_seen"], 2);
    assert_eq!(d["files_written"], 2);
    assert!(d["symbols_written"].as_i64().unwrap() >= 2, "{d}");
    assert!(d["edges_written"].as_i64().unwrap() >= 2, "{d}");
    let after_first = counts(&owner, G).await;
    assert!(after_first.0 > 0 && after_first.1 > 0);

    let env = s.call(&req).await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["files_skipped"], 2, "EC-3: the hashes matched");
    assert_eq!(d["files_written"], 0);
    assert_eq!(
        counts(&owner, G).await,
        after_first,
        "R8: an unchanged re-ingest writes no history"
    );
}

/// FR1: a graph that does not exist is NotFound, because creating one
/// as a side effect would make a typo a new graph.
#[tokio::test]
async fn ingesting_into_an_absent_graph_is_not_found() {
    const G: &str = "iv_absent";
    let Some((owner, s)) = fixtures(G, "iv-absent").await else {
        return;
    };
    let root = tree();
    let env = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: root.path().to_string_lossy().to_string(),
            graph: "iv_never_created".into(),
            overwrite: true,
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("not-found"), "{msg}");
    assert!(
        msg.contains("create it before"),
        "it says what to do: {msg}"
    );
    let exists: bool = owner
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM graphs WHERE name = 'iv_never_created')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists, "the refusal created nothing");
}

/// FR3, EC-1, EC-2: the path is checked before anything is opened, and
/// an empty tree is a summary of zeros rather than an error.
#[tokio::test]
async fn the_path_is_checked_and_an_empty_tree_is_not_an_error() {
    const G: &str = "iv_paths";
    let Some((_o, s)) = fixtures(G, "iv-paths").await else {
        return;
    };
    let d = TempDir::new().unwrap();
    let file = d.path().join("a.rs");
    std::fs::write(&file, "pub fn a() {}\n").unwrap();

    for (path, what) in [
        (file.to_string_lossy().to_string(), "not a directory"),
        ("/nonexistent/tree".to_string(), "No such file"),
    ] {
        let env = s
            .call(&Verb::CodebaseIngest(IngestRequest {
                path,
                graph: G.into(),
                overwrite: true,
            }))
            .await;
        assert!(!env.success);
        let msg = env.error.unwrap();
        assert!(msg.starts_with("invalid-args"), "{msg}");
        assert!(msg.contains(what), "{msg}");
    }

    let empty = TempDir::new().unwrap();
    let env = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: empty.path().to_string_lossy().to_string(),
            graph: G.into(),
            overwrite: true,
        }))
        .await;
    assert!(env.success, "EC-1: nothing to ingest is a fact");
    assert_eq!(data(&env)["files_seen"], 0);
}

/// FR2: documents and then the conforms links, both reported.
#[tokio::test]
async fn ingest_carries_documents_and_their_conforms_links() {
    const G: &str = "iv_documents";
    let Some((_o, s)) = fixtures(G, "iv-documents").await else {
        return;
    };
    let d = TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("docs")).unwrap();
    std::fs::write(
        d.path().join("docs/spec.md"),
        "# Spec\n\nProse.\n\n```graph\nnode: a-claim\nkind: assertion\ntag: review\n```\n",
    )
    .unwrap();
    std::fs::write(
        d.path().join("lib.rs"),
        "//! conforms: a-claim\n\npub fn f() {}\n",
    )
    .unwrap();
    let path = d.path().to_string_lossy().to_string();

    // The code first, so the conforms pass has a file node to link from.
    let env = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);

    let env = s
        .call(&Verb::Ingest(IngestRequest {
            path,
            graph: G.into(),
            overwrite: true,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["documents"]["files_written"], 1);
    assert_eq!(d["documents"]["nodes_declared"], 1, "the claim");
    assert_eq!(d["documents"]["blocks_refused"], 0);
    assert_eq!(d["conforms"]["headers_seen"], 1);
    assert_eq!(d["conforms"]["edges_written"], 1);
    assert_eq!(
        d["conforms"]["unresolved"],
        json!([]),
        "the claim was declared, so the header resolved"
    );
}

/// FR4, EC-4, EC-5: drift reports the three categories and writes
/// nothing.
#[tokio::test]
async fn drift_reports_what_moved_and_changes_nothing() {
    const G: &str = "iv_drift";
    let Some((owner, s)) = fixtures(G, "iv-drift").await else {
        return;
    };
    let root = tree();
    let path = root.path().to_string_lossy().to_string();
    let drift = Verb::CodebaseDrift(DriftRequest {
        graph: G.into(),
        path: path.clone(),
    });

    // EC-4: nothing ingested yet, so everything is new.
    let env = s.call(&drift).await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["clean"], false);
    assert_eq!(d["new"].as_array().unwrap().len(), 2);
    assert_eq!(d["missing"], json!([]));
    assert_eq!(counts(&owner, G).await, (0, 0, 0), "drift wrote nothing");

    s.call(&Verb::CodebaseIngest(IngestRequest {
        path: path.clone(),
        graph: G.into(),
        overwrite: true,
    }))
    .await;
    let before = counts(&owner, G).await;

    // Clean right after an ingest.
    let env = s.call(&drift).await;
    let d = data(&env);
    assert_eq!(d["clean"], true, "{d}");
    assert_eq!(d["unchanged"], 2);

    // One file edited, one deleted.
    std::fs::write(
        root.path().join("helper.rs"),
        "pub fn compute(values: &[i64]) -> i64 {\n    values.iter().product()\n}\n\npub fn extra() {}\n",
    )
    .unwrap();
    std::fs::remove_file(root.path().join("main.rs")).unwrap();

    let env = s.call(&drift).await;
    let d = data(&env);
    assert_eq!(d["clean"], false);
    assert_eq!(d["changed"], json!(["helper.rs"]), "{d}");
    assert_eq!(
        d["missing"],
        json!(["main.rs"]),
        "EC-5, what retire acts on"
    );
    assert_eq!(d["new"], json!([]));
    assert_eq!(
        counts(&owner, G).await,
        before,
        "FR4: drift is a question, not a change"
    );
}

/// FR5 and EC-6: validate is clean on a real graph and finds a planted
/// cross-graph edge, which is the invariant the constraints cannot
/// express.
#[tokio::test]
async fn validate_finds_what_the_constraints_cannot_express() {
    const G: &str = "iv_validate";
    const OTHER: &str = "iv_validate_other";
    let Some((owner, s)) = fixtures(G, "iv-validate").await else {
        return;
    };
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&OTHER])
        .await
        .unwrap();
    owner
        .execute("INSERT INTO graphs (name) VALUES ($1)", &[&OTHER])
        .await
        .unwrap();

    let check = Verb::CodebaseValidate(GraphScoped { graph: G.into() });
    let env = s.call(&check).await;
    assert!(env.success, "{:?}", env.error);
    assert_eq!(data(&env)["ok"], true, "EC-6: an empty graph is clean");

    let root = tree();
    s.call(&Verb::CodebaseIngest(IngestRequest {
        path: root.path().to_string_lossy().to_string(),
        graph: G.into(),
        overwrite: true,
    }))
    .await;
    let env = s.call(&check).await;
    assert_eq!(
        data(&env)["ok"],
        true,
        "a real ingest is clean: {}",
        data(&env)
    );

    // The hole: the foreign keys point at nodes(id) and say nothing
    // about graph_id, so an edge can leave its own graph.
    owner
        .execute(
            "INSERT INTO nodes (graph_id, natural_key, kind)
             SELECT id, 'stranger', 'callable' FROM graphs WHERE name = $1",
            &[&OTHER],
        )
        .await
        .unwrap();
    owner
        .execute(
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer)
             SELECT g.id, n.id, stranger.id, 'calls', 'structural', 'planted'
             FROM graphs g
             JOIN nodes n ON n.graph_id = g.id AND n.kind = 'callable'
             JOIN nodes stranger ON stranger.natural_key = 'stranger'
             WHERE g.name = $1 LIMIT 1",
            &[&G],
        )
        .await
        .unwrap();

    let env = s.call(&check).await;
    let d = data(&env);
    assert_eq!(d["ok"], false, "{d}");
    assert_eq!(d["cross_graph_edges"]["count"], 1);
    assert_eq!(
        d["cross_graph_edges"]["sample"].as_array().unwrap().len(),
        1,
        "the sample names what to look at"
    );
    assert!(
        d["cross_graph_edges"]["sample"][0]
            .as_str()
            .unwrap()
            .contains("stranger"),
        "{d}"
    );

    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&OTHER])
        .await
        .unwrap();
}

/// EC-7: a session with no endpoint cannot open the connection an
/// ingest needs, and says so rather than failing obscurely.
#[tokio::test]
async fn an_ingest_without_an_endpoint_says_so() {
    let Some(dir) = socket_dir() else { return };
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        return;
    };
    let s = Session::new(app, "iv-no-endpoint").with_graph("yeomna_self");
    let root = tree();
    let env = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: root.path().to_string_lossy().to_string(),
            graph: "yeomna_self".into(),
            overwrite: false,
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("internal") && msg.contains("endpoint"),
        "{msg}"
    );
}

/// The daemon spawns one task per connection, so a session's call must
/// be `Send`. A boxed future in the Go language-server path was not,
/// which made every caller of `ingest_codebase` unspawnable, and
/// nothing noticed while ingest was only ever awaited from a test.
/// This is a compile-time guard against its return.
#[test]
fn a_call_is_spawnable() {
    fn require_send<F: Send>(_: F) {}
    fn guard(s: &Session, v: &Verb) {
        require_send(s.call(v));
    }
    let _ = guard;
}
