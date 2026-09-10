//! G1 as a permanent test: every verb call leaves exactly one audit row.
//!
//! The charter says human and agent are logged the same way and does not
//! carve reads out, so this runs over every verb this phase implements
//! rather than over a sample. When a later phase adds a verb, it belongs
//! in `phase_two_verbs` below or the count assertion notices.

use tokio_postgres::Client;
use yeomna_verbs::*;

const PORT: u16 = 5433;
// Each test names its own actor. These run concurrently in one binary
// and each clears its own rows, so a shared actor meant one test's DELETE
// wiped another's rows mid-count, a race that wins often enough to look
// green when the binary runs alone.

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

async fn fixtures(actor: &str) -> Option<(Client, Session)> {
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
    // Shared by every test here and created once, so no test drops a
    // graph another is reading.
    owner
        .execute(
            "INSERT INTO graphs (name) VALUES ('audit_graph') ON CONFLICT (name) DO NOTHING",
            &[],
        )
        .await
        .unwrap();
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    Some((owner, Session::new(app, actor).with_graph("audit_graph")))
}

/// One call of every verb this phase implements, successes and failures
/// mixed, because both must be recorded.
fn phase_two_verbs() -> Vec<Verb> {
    vec![
        Verb::Orient(OrientRequest {
            graph: Some("audit_graph".into()),
        }),
        Verb::Status(Empty {}),
        Verb::Health(Empty {}),
        Verb::Check(CheckRequest {
            key: "nothing".into(),
        }),
        Verb::Stats(StatsRequest { graph: None }),
        Verb::CodebaseStats(GraphScoped {
            graph: "audit_graph".into(),
        }),
        // A miss, so a failure is audited alongside the successes.
        Verb::Get(KindKey {
            kind: "document".into(),
            key: "absent".into(),
        }),
        Verb::List(ListRequest {
            kind: None,
            limit: 5,
            offset: 0,
            parent: None,
        }),
        Verb::Count(CountRequest { kind: None }),
        Verb::Recent(RecentRequest { limit: 5 }),
        Verb::Query(QueryRequest {
            search_text: "anything".into(),
            limit: 5,
            kind: None,
            hybrid: false,
            structural: false,
        }),
        Verb::SchemaVersion(Empty {}),
        // -- Phase 3 (spec 013). Chosen so auditing is proven without
        // side effects: refusals and misses are audited the same as
        // successes, and none of these mutates the cluster.
        Verb::GraphList(Empty {}),
        Verb::GraphTraverse(TraverseRequest {
            graph: "audit_graph".into(),
            start: "absent".into(), // NotFound, audited as failed
            relations: vec![],
            bases: vec![],
            depth: 5,
            limit: 100,
        }),
        Verb::GraphNeighbors(NeighborsRequest {
            graph: "audit_graph".into(),
            key: "absent".into(),
            direction: Direction::Both,
            relations: vec![],
            bases: vec![],
            limit: 10,
        }),
        Verb::GraphShortestPath(ShortestPathRequest {
            graph: "audit_graph".into(),
            from: "a".into(),
            to: "b".into(),
            relations: vec![],
            bases: vec![],
            cap: 100,
        }),
        Verb::GraphCreate(GraphName {
            name: "audit_graph".into(), // exists, InvalidArgs
        }),
        Verb::GraphDrop(DropRequest {
            name: "audit_graph".into(),
            force: false, // Denied, and the refusal is on the record
        }),
        Verb::GraphMaterialize(GraphScoped {
            graph: "audit_graph".into(), // R14's refusal, audited
        }),
        Verb::DatabaseList(Empty {}),
        Verb::DatabaseCreate(DatabaseCreateRequest {
            name: "Bad-Name".into(), // InvalidArgs before any SQL
            kind: DatabaseKind::Plain,
        }),
        Verb::DatabaseDrop(DropRequest {
            name: "audit_db_never".into(),
            force: false, // Denied at the force gate
        }),
        // -- Phase 4 (spec 014). Same principle: refusals and misses
        // audit like successes, and none of these mutates anything.
        Verb::Insert(WriteRequest {
            kind: "no_such_kind".into(), // InvalidArgs before any SQL
            key: "never".into(),
            payload: serde_json::json!({}),
        }),
        Verb::Update(WriteRequest {
            kind: "document".into(),
            key: "absent".into(), // NotFound, the transaction rolls back
            payload: serde_json::json!({}),
        }),
        Verb::Delete(KindKey {
            kind: "document".into(),
            key: "absent".into(), // NotFound
        }),
        Verb::Purge(PurgeRequest {
            key: "absent".into(),
            force: false, // Denied at the force gate
        }),
        Verb::EdgeAssert(EdgeAssertRequest {
            from: "absent".into(), // NotFound names the missing side
            to: "also_absent".into(),
            relation: "depends_on".into(),
            payload: serde_json::Value::Null,
        }),
        Verb::EdgeRetract(EdgeRetractRequest {
            from: "absent".into(), // NotFound, no such asserted edge
            to: "also_absent".into(),
            relation: "depends_on".into(),
        }),
        Verb::Sql(SqlRequest {
            database: "postgres".into(), // Denied by name before any SQL
            statement: "SELECT 1".into(),
        }),
        // -- Phase 6a (spec 019). Refusals again, so auditing is proven
        // without an ingest running inside the completeness sweep.
        Verb::CodebaseIngest(IngestRequest {
            path: "/nonexistent/tree".into(), // InvalidArgs before any connection
            graph: "audit_graph".into(),
            overwrite: false,
        }),
        Verb::Ingest(IngestRequest {
            path: "/nonexistent/tree".into(), // InvalidArgs
            graph: "audit_graph".into(),
            overwrite: false,
        }),
        Verb::CodebaseDrift(DriftRequest {
            graph: "audit_graph".into(),
            path: "/nonexistent/tree".into(), // InvalidArgs
        }),
        Verb::CodebaseValidate(GraphScoped {
            graph: "audit_graph".into(), // clean and side-effect free
        }),
        // -- Phase 6b (spec 020). Both force-gated, so both refuse here
        // and the refusal is what the row records.
        Verb::CodebaseRetire(RetireRequest {
            graph: "audit_graph".into(),
            prefix: "nothing/".into(),
            path: "/nonexistent/tree".into(),
            force: false, // Denied at the force gate
        }),
        Verb::CodebasePrune(DropScoped {
            graph: "audit_graph".into(),
            force: false, // Denied at the force gate
        }),
    ]
}

async fn rows_for(
    owner: &Client,
    verb: &str,
    actor: &str,
) -> Vec<(String, Option<String>, String)> {
    owner
        .query(
            "SELECT actor, outcome, args::text FROM audit_log
             WHERE verb = $1 AND actor = $2 ORDER BY id",
            &[&verb, &actor],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect()
}

#[tokio::test]
async fn every_verb_call_leaves_exactly_one_terminal_row() {
    const ACTOR: &str = "audit-every-verb";
    let Some((owner, s)) = fixtures(ACTOR).await else {
        eprintln!("SKIP: no cluster");
        return;
    };
    // Start from a clean slate for this actor. audit_log is append-only
    // for the app role, and the owner is doing the cleaning.
    owner
        .execute("DELETE FROM audit_log WHERE actor = $1", &[&ACTOR])
        .await
        .unwrap();

    for verb in phase_two_verbs() {
        let name = verb.wire_name();
        let before = rows_for(&owner, name, ACTOR).await.len();
        let env = s.call(&verb).await;
        let after = rows_for(&owner, name, ACTOR).await;

        // Both counts, not their difference: a subtraction in the
        // message can underflow and panic in place of the diagnostic it
        // was meant to print.
        assert_eq!(
            after.len(),
            before + 1,
            "{name} must leave exactly one row, had {before} and now has {}",
            after.len()
        );
        let (actor, outcome, args) = after.last().unwrap();
        assert_eq!(actor, ACTOR, "{name} recorded the wrong actor");

        // V-Q1: terminal, and the failure mark carries the taxonomy's
        // kind rather than the error's detail.
        let outcome = outcome.as_deref().expect("the outcome is marked");
        if env.success {
            assert_eq!(outcome, "ok", "{name}");
        } else {
            assert!(outcome.starts_with("failed: "), "{name}: {outcome}");
            let kind = outcome.trim_start_matches("failed: ");
            assert!(
                env.error.as_ref().unwrap().starts_with(kind),
                "{name}: the row says {kind:?} and the envelope says {:?}",
                env.error
            );
            assert!(
                !outcome.contains(':') || outcome.matches(':').count() == 1,
                "{name}: the column takes the kind, not the detail: {outcome}"
            );
        }

        // The args are what the caller asked for, and carry no actor,
        // since the contract has no such field (V3).
        assert!(
            !args.contains("\"actor\""),
            "{name} leaked an actor into args"
        );
    }
}

#[tokio::test]
async fn the_args_recorded_are_the_request_the_caller_made() {
    const ACTOR: &str = "audit-args";
    let Some((owner, s)) = fixtures(ACTOR).await else {
        return;
    };
    owner
        .execute("DELETE FROM audit_log WHERE actor = $1", &[&ACTOR])
        .await
        .unwrap();
    s.call(&Verb::Query(QueryRequest {
        search_text: "a distinctive phrase".into(),
        limit: 7,
        kind: Some("document".into()),
        hybrid: false,
        structural: false,
    }))
    .await;
    let rows = rows_for(&owner, "query", ACTOR).await;
    let args: serde_json::Value = serde_json::from_str(&rows.last().unwrap().2).unwrap();
    assert_eq!(args["search_text"], "a distinctive phrase");
    assert_eq!(args["limit"], 7);
    assert_eq!(args["kind"], "document");
}

/// EC-5: a call that cannot be recorded is a call the appliance declines
/// to make.
///
/// Proven on a read-only connection rather than by revoking a grant. The
/// first version of this test did revoke `INSERT ON audit_log`, which is
/// cluster-wide state: it passed alone and broke every sibling test that
/// called a verb inside the window. A session-local setting isolates the
/// failure to this connection and nothing else sees it.
#[tokio::test]
async fn a_verb_does_not_run_when_the_log_refuses_the_attempt() {
    let Some(dir) = socket_dir() else { return };
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        return;
    };
    app.batch_execute("SET default_transaction_read_only = on")
        .await
        .expect("session-local, so no other connection is affected");
    let s = Session::new(app, "audit-readonly");

    let env = s.call(&Verb::Status(Empty {})).await;

    assert!(!env.success, "the verb ran without being recorded");
    let msg = env.error.unwrap();
    assert!(msg.starts_with("internal"), "{msg}");
    assert!(msg.contains("audit"), "it says why: {msg}");
}

/// V3 end to end: a session built with the kernel's actor writes that
/// actor, and nothing in the environment changes it. Lives here because
/// reading the audit log is SQL, and this is one of the two crates that
/// may write it.
#[tokio::test]
async fn the_kernel_s_actor_is_what_the_row_carries() {
    let actor = yeomna_verbs::actor::from_kernel();
    let Some((owner, _)) = fixtures(&actor).await else {
        return;
    };
    let Some(dir) = socket_dir() else { return };
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app connects");
    owner
        .execute("DELETE FROM audit_log WHERE actor = $1", &[&actor])
        .await
        .unwrap();

    // The environment is not consulted, which the CLI suite proves at
    // process level with a hostile USER. Mutating it here would be a
    // process-wide change in a binary whose tests run in parallel, so
    // this test asserts the derivation and leaves the environment alone.
    let session = Session::new(app, yeomna_verbs::actor::from_kernel());
    let env = session.call(&Verb::Health(Empty {})).await;
    assert!(env.success, "{:?}", env.error);

    let rows = rows_for(&owner, "health", &actor).await;
    assert_eq!(
        rows.len(),
        1,
        "the call is audited under the kernel's actor"
    );
    assert_eq!(rows[0].1.as_deref(), Some("ok"));
    assert!(
        !actor.is_empty() && actor != "impostor" && actor != "unknown",
        "the kernel named the caller: {actor:?}"
    );
}
