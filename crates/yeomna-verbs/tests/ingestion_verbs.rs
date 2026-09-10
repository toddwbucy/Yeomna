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
        embed: false,
        embed_task: None,
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
            embed: false,
            embed_task: None,
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
                embed: false,
                embed_task: None,
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
            embed: false,
            embed_task: None,
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
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(env.success, "the seeding ingest: {:?}", env.error);

    let env = s
        .call(&Verb::Ingest(IngestRequest {
            path,
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
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

    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "the seeding ingest: {:?}", seed.error);
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
    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: root.path().to_string_lossy().to_string(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "the seeding ingest: {:?}", seed.error);
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
    // The graph has to exist for the endpoint check to be the one that
    // fires, because `sink_for` looks for the graph first. This used to
    // borrow the dogfood graph `yeomna_self`, so the test passed only on a
    // machine that happened to have one and would have failed on a fresh
    // appliance or after the drop a schema bump costs. It owns its graph
    // now, like every other test in this file.
    let Some((_owner, scaffold)) = fixtures("iv_no_endpoint", "iv-scaffold").await else {
        return;
    };
    drop(scaffold);
    let dir = socket_dir().expect("fixtures proved it");
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        return;
    };
    let s = Session::new(app, "iv-no-endpoint").with_graph("iv_no_endpoint");
    let root = tree();
    let env = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: root.path().to_string_lossy().to_string(),
            graph: "iv_no_endpoint".into(),
            overwrite: false,
            embed: false,
            embed_task: None,
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

/// The finding this test exists for: a file the walk offered and could
/// not assess is present, not absent, and reporting it as `missing`
/// would tell `retire` to sweep a node whose source is right there.
///
/// Both skip paths are exercised: over the size limit, and unreadable
/// as text. Each leaves `missing` empty, lands in `unassessed`, and
/// makes `clean` false, because a drift that could not read a file
/// cannot answer yes.
#[tokio::test]
async fn a_file_the_walk_cannot_assess_is_not_reported_missing() {
    const G: &str = "iv_unassessed";
    let Some((owner, s)) = fixtures(G, "iv-unassessed").await else {
        return;
    };
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("small.rs"), "pub fn a() {}\n").unwrap();
    std::fs::write(root.path().join("big.rs"), "pub fn b() {}\n").unwrap();
    let path = root.path().to_string_lossy().to_string();

    // Ingest both while they are readable and small, so the graph holds
    // a node for each.
    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "the seeding ingest: {:?}", seed.error);
    assert_eq!(data(&seed)["files_written"], 2);
    let before = counts(&owner, G).await;

    // Now make one oversized and one unreadable as text, without
    // removing either from the tree.
    let big = "pub fn b() {}\n".repeat(100_000);
    assert!(big.len() > 1024 * 1024, "the fixture must exceed the limit");
    std::fs::write(root.path().join("big.rs"), &big).unwrap();
    std::fs::write(root.path().join("small.rs"), [0xff, 0xfe, 0x00, 0x01]).unwrap();

    let env = s
        .call(&Verb::CodebaseDrift(DriftRequest {
            graph: G.into(),
            path: path.clone(),
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(
        d["missing"],
        json!([]),
        "both files are present, so neither is missing: {d}"
    );
    assert_eq!(d["changed"], json!([]));
    assert_eq!(d["new"], json!([]));
    assert_eq!(d["files_seen"], 2, "the walk saw both: {d}");
    let unassessed: Vec<String> = d["unassessed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(unassessed.len(), 2, "{unassessed:?}");
    assert!(
        unassessed
            .iter()
            .any(|u| u.starts_with("big.rs: over the size limit")),
        "{unassessed:?}"
    );
    assert!(
        unassessed
            .iter()
            .any(|u| u.starts_with("small.rs: could not be read")),
        "{unassessed:?}"
    );
    assert_eq!(
        d["clean"], false,
        "a drift that could not read a file does not answer yes"
    );
    assert_eq!(counts(&owner, G).await, before, "drift still wrote nothing");
}

/// FR1 through FR4 and EC-1, EC-2, EC-6: retire sweeps what the tree no
/// longer has, refuses without force, refuses a wildcard, and refuses a
/// tree it cannot walk.
#[tokio::test]
async fn retire_sweeps_what_the_source_lost_and_refuses_the_rest() {
    const G: &str = "iv_retire";
    let Some((owner, s)) = fixtures(G, "iv-retire").await else {
        return;
    };
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("old")).unwrap();
    std::fs::write(root.path().join("keep.rs"), "pub fn keep() {}\n").unwrap();
    std::fs::write(
        root.path().join("old/gone.rs"),
        "pub fn gone() -> i64 {\n    1\n}\n",
    )
    .unwrap();
    let path = root.path().to_string_lossy().to_string();
    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "the seeding ingest: {:?}", seed.error);
    let before = counts(&owner, G).await;

    let retire = |prefix: &str, tree: &str, force: bool| {
        Verb::CodebaseRetire(RetireRequest {
            graph: G.into(),
            prefix: prefix.into(),
            path: tree.into(),
            force,
        })
    };

    // FR1: no force, no sweep.
    let env = s.call(&retire("old/", &path, false)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("denied"));
    assert_eq!(counts(&owner, G).await, before, "the refusal swept nothing");

    // EC-6: an empty prefix would name the whole graph.
    let env = s.call(&retire("   ", &path, true)).await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("invalid-args") && msg.contains("every file"),
        "{msg}"
    );

    // EC-2: a tree that cannot be walked would make every file look
    // gone, and that is a refusal rather than a licence to sweep.
    let env = s.call(&retire("old/", "/nonexistent/tree", true)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("invalid-args"));
    assert_eq!(counts(&owner, G).await, before, "still nothing swept");

    // EC-1: the files are all still there, so there is nothing to
    // retire even with force.
    let env = s.call(&retire("old/", &path, true)).await;
    assert!(env.success, "{:?}", env.error);
    assert_eq!(data(&env)["retired"], json!([]), "{}", data(&env));
    assert_eq!(data(&env)["swept"]["nodes"], 0);
    assert_eq!(counts(&owner, G).await, before);

    // FR2: now the file is gone, and retire takes its family.
    std::fs::remove_file(root.path().join("old/gone.rs")).unwrap();
    let env = s.call(&retire("old/", &path, true)).await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["retired"], json!(["old/gone.rs"]), "{d}");
    assert!(
        d["swept"]["nodes"].as_i64().unwrap() >= 2,
        "the file node and the symbol it declared: {d}"
    );
    let after = counts(&owner, G).await;
    assert!(after.0 < before.0, "nodes went: {before:?} then {after:?}");

    // The file outside the prefix is untouched, and so is the graph's
    // record of it.
    let env = s
        .call(&Verb::CodebaseDrift(DriftRequest {
            graph: G.into(),
            path: path.clone(),
        }))
        .await;
    let d = data(&env);
    assert_eq!(d["clean"], true, "what remains matches the tree: {d}");
    assert_eq!(
        d["missing"],
        json!([]),
        "the retired file is no longer known"
    );
}

/// FR3: a file that is present and could not be assessed is never
/// retired. This is the guard spec 019's round one made necessary, and
/// it is the one that would have deleted live knowledge.
#[tokio::test]
async fn retire_never_touches_a_present_file_it_could_not_assess() {
    const G: &str = "iv_retire_unassessed";
    let Some((owner, s)) = fixtures(G, "iv-retire-unassessed").await else {
        return;
    };
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/big.rs"), "pub fn b() {}\n").unwrap();
    let path = root.path().to_string_lossy().to_string();
    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "{:?}", seed.error);
    let before = counts(&owner, G).await;
    assert!(before.0 > 0);

    // Present, and past the size limit, so drift cannot assess it.
    std::fs::write(
        root.path().join("src/big.rs"),
        "pub fn b() {}\n".repeat(100_000),
    )
    .unwrap();

    let env = s
        .call(&Verb::CodebaseRetire(RetireRequest {
            graph: G.into(),
            prefix: "src/".into(),
            path: path.clone(),
            force: true,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    assert_eq!(
        data(&env)["retired"],
        json!([]),
        "a present file is not gone, whatever the analyzer could do with it"
    );
    assert_eq!(
        counts(&owner, G).await,
        before,
        "the graph's record of source that is still there survived"
    );
}

/// FR5 and EC-3, EC-4: prune sweeps the orphans a retire would leave if
/// it ever left any, refuses without force, and finds nothing after a
/// retire that took the family.
#[tokio::test]
async fn prune_sweeps_orphans_and_finds_none_after_a_clean_retire() {
    const G: &str = "iv_prune";
    let Some((owner, s)) = fixtures(G, "iv-prune").await else {
        return;
    };
    let root = tree();
    let path = root.path().to_string_lossy().to_string();
    let seed = s
        .call(&Verb::CodebaseIngest(IngestRequest {
            path: path.clone(),
            graph: G.into(),
            overwrite: true,
            embed: false,
            embed_task: None,
        }))
        .await;
    assert!(seed.success, "{:?}", seed.error);

    let prune = |force: bool| {
        Verb::CodebasePrune(DropScoped {
            graph: G.into(),
            force,
        })
    };

    // FR5: no force, no sweep.
    let env = s.call(&prune(false)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("denied"));

    // EC-3: a real ingest leaves no orphans.
    let env = s.call(&prune(true)).await;
    assert!(env.success, "{:?}", env.error);
    assert_eq!(data(&env)["swept"]["nodes"], 0, "{}", data(&env));

    // Plant the orphan class: a file node deleted without its symbols,
    // which is what a half-finished retire would leave.
    owner
        .execute(
            "DELETE FROM nodes n USING graphs g
             WHERE g.id = n.graph_id AND g.name = $1 AND n.kind = 'file'
               AND n.natural_key = 'helper_rs'",
            &[&G],
        )
        .await
        .unwrap();
    let orphans: i64 = owner
        .query_one(
            "SELECT count(*) FROM nodes n JOIN graphs g ON g.id = n.graph_id
             WHERE g.name = $1 AND n.payload->>'file_key' = 'helper_rs'",
            &[&G],
        )
        .await
        .unwrap()
        .get(0);
    assert!(orphans > 0, "the fixture planted orphans");

    let env = s.call(&prune(true)).await;
    assert!(env.success, "{:?}", env.error);
    let d = data(&env);
    assert_eq!(d["swept"]["nodes"], orphans, "{d}");
    assert!(
        !d["sample"].as_array().unwrap().is_empty(),
        "the sample names what went: {d}"
    );

    // EC-4: nothing left to prune.
    let env = s.call(&prune(true)).await;
    assert_eq!(data(&env)["swept"]["nodes"], 0);
}

/// FR8 and the point of the phase: nothing in the dispatch refuses by
/// phase any more, and every destructive verb leaves its audit row.
/// T3's falsifying clause named `codebase retire` producing no
/// verb-layer record. It produces one.
#[tokio::test]
async fn every_destructive_verb_leaves_a_record_of_its_call() {
    const G: &str = "iv_t3";
    const ACTOR: &str = "iv-t3";
    let Some((owner, s)) = fixtures(G, ACTOR).await else {
        return;
    };
    owner
        .execute("DELETE FROM audit_log WHERE actor = $1", &[&ACTOR])
        .await
        .unwrap();
    let root = tree();
    let path = root.path().to_string_lossy().to_string();
    s.call(&Verb::CodebaseIngest(IngestRequest {
        path: path.clone(),
        graph: G.into(),
        overwrite: true,
        embed: false,
        embed_task: None,
    }))
    .await;

    for verb in [
        Verb::CodebaseRetire(RetireRequest {
            graph: G.into(),
            prefix: "helper".into(),
            path: path.clone(),
            force: true,
        }),
        Verb::CodebasePrune(DropScoped {
            graph: G.into(),
            force: true,
        }),
    ] {
        let name = verb.wire_name();
        let env = s.call(&verb).await;
        assert!(env.success, "{name}: {:?}", env.error);
        let row = owner
            .query_one(
                "SELECT actor, outcome, args::text FROM audit_log
                 WHERE actor = $1 AND verb = $2 ORDER BY id DESC LIMIT 1",
                &[&ACTOR, &name],
            )
            .await
            .unwrap_or_else(|e| panic!("{name} left no audit row: {e}"));
        assert_eq!(row.get::<_, String>(0), ACTOR);
        assert_eq!(
            row.get::<_, Option<String>>(1).as_deref(),
            Some("ok"),
            "{name}"
        );
        let args: serde_json::Value = serde_json::from_str(&row.get::<_, String>(2)).unwrap();
        assert_eq!(args["graph"], G, "{name}: the row says which graph");
        assert_eq!(args["force"], true, "{name}: and that force was given");
    }
}

/// R26: one cohort per graph, not one model.
///
/// `validate` used to accept any graph with at most one distinct
/// `embeddings.model`. Two vectors from the same model at another revision
/// or under another LoRA adapter share a name and have incomparable
/// geometry, so that check reported `ok` on a graph whose vectors cannot be
/// ranked against a single query vector. Spec 022's columns are what make
/// the stronger check expressible, and this is the check.
#[tokio::test]
async fn validate_refuses_a_graph_holding_two_cohorts() {
    const G: &str = "iv_cohorts";
    let Some((owner, s)) = fixtures(G, "iv-cohorts").await else {
        return;
    };
    let gid: i64 = owner
        .query_one("SELECT id FROM graphs WHERE name = $1", &[&G])
        .await
        .unwrap()
        .get(0);
    let node: i64 = owner
        .query_one(
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, 'f', 'file', '{\"path\":\"f.rs\",\"symbol_hash\":\"h\"}'::jsonb)
             RETURNING id",
            &[&gid],
        )
        .await
        .unwrap()
        .get(0);
    let literal = format!("[{}]", vec!["0.1"; 2048].join(","));
    for (i, (rev, task)) in [("rev-a", "retrieval.passage"), ("rev-b", "code")]
        .into_iter()
        .enumerate()
    {
        let chunk: i64 = owner
            .query_one(
                "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
                 VALUES ($1, $2, 'text', 0, 4) RETURNING id",
                &[&node, &(i as i32)],
            )
            .await
            .unwrap()
            .get(0);
        owner
            .execute(
                "INSERT INTO embeddings (chunk_id, vec, model, model_hash, model_revision, task)
                 VALUES ($1, $2::text::halfvec, 'same/model', 'h', $3, $4)",
                &[&chunk, &literal, &rev, &task],
            )
            .await
            .unwrap();
    }

    let env = s
        .call(&Verb::CodebaseValidate(GraphScoped { graph: G.into() }))
        .await;
    let d = data(&env);
    assert_eq!(
        d["ok"], false,
        "two cohorts cannot be ranked against one query vector: {d}"
    );
    let cohorts = d["embedding_cohorts"].as_array().unwrap();
    assert_eq!(cohorts.len(), 2, "both are named: {cohorts:?}");
    for c in cohorts {
        let text = c.as_str().unwrap();
        assert!(text.starts_with("same/model @ "), "{text}");
    }
    assert!(
        cohorts
            .iter()
            .any(|c| c.as_str().unwrap().contains("rev-a"))
            && cohorts.iter().any(|c| c.as_str().unwrap().contains("code")),
        "the revision and the task both distinguish a cohort: {cohorts:?}"
    );
}
