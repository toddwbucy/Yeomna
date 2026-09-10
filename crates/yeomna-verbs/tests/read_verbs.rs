//! The read verbs against the live schema, per spec 012.
//!
//! Two corpora on purpose. A scratch graph where the assertion needs to
//! know the exact contents, and `yeomna_self` (the H3 dogfood of this
//! repository) where the point is that the verb survives real data. The
//! dogfood tests skip when that graph is absent, since it is written by
//! an operation rather than by the suite.

use serde_json::{Value, json};
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

/// An owner connection for fixtures and a session over the runtime role.
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
    Some((owner, Session::new(app, "test").with_graph(graph)))
}

/// A small known graph: one document, two chunks, one callable.
async fn seed(c: &Client, graph: &str) {
    let g: i64 = c
        .query_one(
            "INSERT INTO graphs (name) VALUES ($1) RETURNING id",
            &[&graph],
        )
        .await
        .unwrap()
        .get(0);
    let doc: i64 = c
        .query_one(
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, 'docA', 'document', '{\"title\":\"the recursive descent parser\"}')
             RETURNING id",
            &[&g],
        )
        .await
        .unwrap()
        .get(0);
    c.execute(
        "INSERT INTO nodes (graph_id, natural_key, kind, payload)
         VALUES ($1, 'symA', 'callable', '{\"file_key\":\"docA\"}')",
        &[&g],
    )
    .await
    .unwrap();
    for (i, text) in [
        "a recursive descent parser reads tokens left to right",
        "the second chunk mentions postgres and nothing else",
    ]
    .iter()
    .enumerate()
    {
        c.execute(
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
             VALUES ($1, $2, $3, 0, 10)",
            &[&doc, &(i as i32), text],
        )
        .await
        .unwrap();
    }
}

macro_rules! require {
    ($owner:ident, $s:ident, $graph:literal) => {
        let Some(($owner, $s)) = fixtures($graph).await else {
            return;
        };
    };
}

/// Unwrap a successful envelope, or fail loudly with its error.
fn data(e: &Envelope) -> &Value {
    assert!(e.success, "expected success, got {:?}", e.error);
    e.data.as_ref().expect("success carries data")
}

#[tokio::test]
async fn orient_surveys_a_graph_as_v_q3_ruled() {
    require!(_o, s, "verbs_orient");
    let env = s
        .call(&Verb::Orient(OrientRequest {
            graph: Some("verbs_orient".into()),
        }))
        .await;
    let d = data(&env);
    assert_eq!(d["schema_version"], yeomna_store::SCHEMA_VERSION);
    let g = &d["graphs"][0];
    assert_eq!(g["graph"], "verbs_orient");
    assert_eq!(g["nodes_by_kind"]["document"], 1);
    assert_eq!(g["nodes_by_kind"]["callable"], 1);
    assert_eq!(g["chunks"], 2);
    assert_eq!(g["embeddings"], 0, "nothing embeds until H4");
    // `is_some()` here would always hold, since the key is written even
    // when the value is null. The timestamp itself is the claim.
    let last = g["last_ingest"]
        .as_str()
        .expect("R9 made this answerable, and the seed just wrote nodes");
    chrono::DateTime::parse_from_rfc3339(last).expect("an RFC 3339 instant");
}

#[tokio::test]
async fn orient_distinguishes_a_missing_graph_from_an_empty_one() {
    require!(owner, s, "verbs_missing");
    // EC-3: absent is NotFound.
    let env = s
        .call(&Verb::Orient(OrientRequest {
            graph: Some("no_such_graph".into()),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.as_ref().unwrap().starts_with("not-found"));

    // Present but empty is a survey of zeros, not an error. Cleared
    // first and inserted idempotently, because a panic below would
    // otherwise leave the row and the next run would fail on the unique
    // constraint rather than on the behavior under test.
    owner
        .execute("DELETE FROM graphs WHERE name = 'verbs_empty'", &[])
        .await
        .unwrap();
    owner
        .execute(
            "INSERT INTO graphs (name) VALUES ('verbs_empty') ON CONFLICT (name) DO NOTHING",
            &[],
        )
        .await
        .unwrap();
    let env = s
        .call(&Verb::Orient(OrientRequest {
            graph: Some("verbs_empty".into()),
        }))
        .await;
    let d = data(&env);
    assert_eq!(d["graphs"][0]["chunks"], 0);
    owner
        .execute("DELETE FROM graphs WHERE name = 'verbs_empty'", &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn stats_refuses_a_missing_graph_and_list_reports_the_applied_limit() {
    require!(_o, s, "verbs_limits");
    // A misspelled graph is not an empty one, the same rule orient and
    // codebase.stats already hold to.
    let env = s
        .call(&Verb::Stats(StatsRequest {
            graph: Some("no_such_graph".into()),
        }))
        .await;
    assert!(!env.success, "zeros would read as an empty graph");
    assert!(env.error.unwrap().starts_with("not-found"));

    // The reported limit is the applied one, so a client paging until it
    // sees a short page is not told to expect more than it can get.
    let d = data(
        &s.call(&Verb::List(ListRequest {
            kind: None,
            limit: 50_000,
            offset: 0,
            parent: None,
        }))
        .await,
    )
    .clone();
    assert_eq!(d["limit"], 1000, "the cap is reported, not the request");
}

#[tokio::test]
async fn status_and_health_answer_and_report_counts() {
    require!(_o, s, "verbs_status");
    let d = data(&s.call(&Verb::Status(Empty {})).await).clone();
    assert_eq!(d["store"], "answering");
    assert_eq!(d["database"], "yeomna");
    assert_eq!(d["schema_version"], yeomna_store::SCHEMA_VERSION);

    let d = data(&s.call(&Verb::Health(Empty {})).await).clone();
    // Numbers rather than a verdict: the seeded callable has no chunks
    // and every chunk lacks an embedding, and both are normal here.
    assert!(d["chunks_without_embeddings"].as_i64().unwrap() >= 2);
    assert!(d["nodes"].as_i64().unwrap() >= 2);
}

#[tokio::test]
async fn get_list_count_and_recent_read_the_seeded_graph() {
    require!(_o, s, "verbs_read");
    let d = data(
        &s.call(&Verb::Get(KindKey {
            kind: "document".into(),
            key: "docA".into(),
        }))
        .await,
    )
    .clone();
    assert_eq!(d["key"], "docA");
    assert_eq!(d["payload"]["title"], "the recursive descent parser");

    let d = data(&s.call(&Verb::Count(CountRequest { kind: None })).await).clone();
    assert_eq!(d["count"], 2);

    let d = data(
        &s.call(&Verb::Count(CountRequest {
            kind: Some("document".into()),
        }))
        .await,
    )
    .clone();
    assert_eq!(d["count"], 1);

    let d = data(
        &s.call(&Verb::List(ListRequest {
            kind: Some("callable".into()),
            limit: 20,
            offset: 0,
            parent: None,
        }))
        .await,
    )
    .clone();
    assert_eq!(d["nodes"].as_array().unwrap().len(), 1);

    // The parent filter reads the linkage the orchestrator writes.
    let d = data(
        &s.call(&Verb::List(ListRequest {
            kind: None,
            limit: 20,
            offset: 0,
            parent: Some("docA".into()),
        }))
        .await,
    )
    .clone();
    assert_eq!(
        d["nodes"].as_array().unwrap().len(),
        1,
        "symA is under docA"
    );

    let d = data(&s.call(&Verb::Recent(RecentRequest { limit: 10 })).await).clone();
    assert_eq!(d["nodes"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn query_ranks_with_ts_rank_cd_and_refuses_what_it_cannot_do() {
    require!(_o, s, "verbs_query");
    let d = data(
        &s.call(&Verb::Query(QueryRequest {
            search_text: "recursive parser".into(),
            limit: 10,
            kind: None,
            hybrid: false,
            structural: false,
        }))
        .await,
    )
    .clone();
    let hits = d["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "one chunk matches, the other does not");
    assert!(
        hits[0]["rank"].as_f64().unwrap() > 0.0,
        "ts_rank_cd ranked it"
    );
    assert_eq!(hits[0]["key"], "docA");

    // A requested ranking mode that cannot run is refused, not ignored.
    for (hybrid, structural, waits_for) in [(true, false, "H4"), (false, true, "H9")] {
        let env = s
            .call(&Verb::Query(QueryRequest {
                search_text: "parser".into(),
                limit: 10,
                kind: None,
                hybrid,
                structural,
            }))
            .await;
        assert!(!env.success);
        let msg = env.error.unwrap();
        assert!(msg.starts_with("unimplemented"), "{msg}");
        assert!(msg.contains(waits_for), "it names what it waits for: {msg}");
    }

    let env = s
        .call(&Verb::Query(QueryRequest {
            search_text: "   ".into(),
            limit: 10,
            kind: None,
            hybrid: false,
            structural: false,
        }))
        .await;
    assert!(
        !env.success,
        "an empty search is invalid, not empty results"
    );
}

#[tokio::test]
async fn an_unknown_kind_is_refused_rather_than_answered_empty() {
    require!(_o, s, "verbs_kind");
    // FR 5: a kind outside the schema's CHECK set could never match, so
    // an empty result would be a false answer.
    for verb in [
        Verb::Get(KindKey {
            kind: "wizard".into(),
            key: "docA".into(),
        }),
        Verb::Count(CountRequest {
            kind: Some("wizard".into()),
        }),
        Verb::List(ListRequest {
            kind: Some("wizard".into()),
            limit: 5,
            offset: 0,
            parent: None,
        }),
    ] {
        let env = s.call(&verb).await;
        assert!(!env.success, "{} accepted a bogus kind", verb.wire_name());
        assert!(env.error.unwrap().starts_with("invalid-args"));
    }
}

#[tokio::test]
async fn get_reports_ambiguity_rather_than_choosing() {
    let Some(dir) = socket_dir() else { return };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        return;
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
        return;
    }
    yeomna_store::apply_schema(&owner).await.expect("schema");
    for g in ["verbs_amb_a", "verbs_amb_b"] {
        owner
            .execute("DELETE FROM graphs WHERE name = $1", &[&g])
            .await
            .unwrap();
        let id: i64 = owner
            .query_one("INSERT INTO graphs (name) VALUES ($1) RETURNING id", &[&g])
            .await
            .unwrap()
            .get(0);
        owner
            .execute(
                "INSERT INTO nodes (graph_id, natural_key, kind) VALUES ($1, 'shared', 'document')",
                &[&id],
            )
            .await
            .unwrap();
    }
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .unwrap();
    // No session graph, and the key lives in two: EC-2.
    let s = Session::new(app, "test");
    let env = s
        .call(&Verb::Get(KindKey {
            kind: "document".into(),
            key: "shared".into(),
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("invalid-args"), "{msg}");
    assert!(
        msg.contains("verbs_amb_a") && msg.contains("verbs_amb_b"),
        "{msg}"
    );
    for g in ["verbs_amb_a", "verbs_amb_b"] {
        owner
            .execute("DELETE FROM graphs WHERE name = $1", &[&g])
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_later_phase_verb_names_the_phase_it_waits_for() {
    require!(_o, s, "verbs_later");
    // EC-1: reachable through dispatch, refused by name, never a panic.
    //
    // This list shrank as the phases landed, and spec 020 emptied it of
    // them: 013 took the graph and database verbs, 014 the writes and
    // sql, 019 the ingesting half of Phase 6, 020 the destructive half.
    // **No verb refuses by phase any more.** What is left waits on a
    // hole (H4's embedder, H7's schema manager, H9's graph embeddings),
    // which is a capability the appliance does not have rather than a
    // surface that routes around the verb layer, and that distinction is
    // what T3's clause turns on.
    let cases = [
        (Verb::SchemaShow(Empty {}), "H7"),
        (
            Verb::EmbedText(EmbedTextRequest {
                text: "hello".into(),
                task: None,
            }),
            "H4",
        ),
        (
            Verb::GraphEmbedEmbed(GraphKey {
                graph: "verbs_later".into(),
                key: "anything".into(),
            }),
            "H9",
        ),
    ];
    for (verb, phase) in cases {
        let env = s.call(&verb).await;
        assert!(!env.success, "{} should be unimplemented", verb.wire_name());
        let msg = env.error.unwrap();
        assert!(msg.starts_with("unimplemented"), "{msg}");
        assert!(msg.contains(phase), "names its phase: {msg}");
    }
}

#[tokio::test]
async fn schema_version_reports_the_compiled_in_constant() {
    require!(_o, s, "verbs_schemaver");
    let d = data(&s.call(&Verb::SchemaVersion(Empty {})).await).clone();
    assert_eq!(d["version"], yeomna_store::SCHEMA_VERSION);
}

/// The dogfood graph, where the point is surviving real data rather than
/// knowing the contents. Skips when H3's operation has not been run.
#[tokio::test]
async fn the_verbs_answer_over_the_dogfood_graph() {
    let Some(dir) = socket_dir() else { return };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        return;
    };
    // The gate covers what the assertions below need, which is chunks
    // with a populated tsv, not merely a graph row. An ingest that wrote
    // nodes and no chunks would otherwise fail the FTS assertion for a
    // fixture reason rather than a verb defect.
    let usable: bool = owner
        .query_one(
            "SELECT EXISTS (
               SELECT 1 FROM chunks c
                 JOIN nodes n ON n.id = c.node_id
                 JOIN graphs g ON g.id = n.graph_id
               WHERE g.name = 'yeomna_self')",
            &[],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(false);
    if !usable {
        eprintln!("SKIP: no chunked yeomna_self graph, run the H3 dogfood operation");
        return;
    }
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .unwrap();
    let s = Session::new(app, "test").with_graph("yeomna_self");

    let d = data(
        &s.call(&Verb::CodebaseStats(GraphScoped {
            graph: "yeomna_self".into(),
        }))
        .await,
    )
    .clone();
    assert!(
        d["nodes_by_kind"]["callable"].as_i64().unwrap() > 100,
        "the real graph has real symbols: {d}"
    );
    assert!(
        d["edges_by_relation"]["defines"].as_i64().unwrap() > 100,
        "and real edges: {d}"
    );
    assert!(
        d["analyzers"].as_object().unwrap().contains_key("syn"),
        "attributed to the analyzer that ran: {d}"
    );

    // FTS over a corpus that was never written for this test.
    let d = data(
        &s.call(&Verb::Query(QueryRequest {
            search_text: "sink".into(),
            limit: 5,
            kind: None,
            hybrid: false,
            structural: false,
        }))
        .await,
    )
    .clone();
    assert!(
        !d["hits"].as_array().unwrap().is_empty(),
        "this repository talks about sinks: {d}"
    );
}

/// A shape the envelope promises and every caller depends on.
#[tokio::test]
async fn every_response_is_a_well_formed_envelope() {
    require!(_o, s, "verbs_envelope");
    for verb in [
        Verb::Status(Empty {}),
        Verb::Count(CountRequest { kind: None }),
        Verb::Get(KindKey {
            kind: "document".into(),
            key: "absent".into(),
        }),
    ] {
        let env = s.call(&verb).await;
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["command"], verb.wire_name());
        assert!(v["timestamp"].is_string());
        chrono::DateTime::parse_from_rfc3339(v["timestamp"].as_str().unwrap()).unwrap();
        if env.success {
            assert!(v.get("error").is_none());
        } else {
            assert!(v.get("data").is_none());
        }
    }
    // FR 1 in one line: a miss is a typed failure, not a panic.
    let env = s
        .call(&Verb::Get(KindKey {
            kind: "document".into(),
            key: "absent".into(),
        }))
        .await;
    assert_eq!(
        env.error.as_deref().map(|e| e.starts_with("not-found")),
        Some(true)
    );
    let _ = json!({});
}
