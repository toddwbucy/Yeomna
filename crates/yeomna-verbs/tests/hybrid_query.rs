//! Hybrid query: reciprocal rank fusion over keyword and vector (spec 023).
//!
//! Gated twice and separately, because a hybrid query needs both Postgres
//! and the embedder and a run missing one should say which. Every skip
//! names its cause.
//!
//! The corpus here is embedded by the real service rather than by a double.
//! A fused ranking is a claim about two rankings agreeing, and a double's
//! vectors would let the vector half agree with nothing in particular. The
//! chunks are chosen so one matches the query lexically, one matches it by
//! meaning and shares no term, and one matches neither, which is the whole
//! of what fusion is for.

use serde_json::{Value, json};
use tokio_postgres::Client;
use yeomna_embed::embedding::EmbeddingClient;
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

fn embedder_socket() -> Option<String> {
    let path = std::env::var("YEOMNA_EMBEDDER_SOCKET").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run/embedder.sock")
    });
    std::path::Path::new(&path).exists().then_some(path)
}

/// The lexical hit: shares terms with the query.
const LEXICAL: &str = "The recursive descent parser reads tokens from left to right, \
                       one token at a time, and never backtracks past a decision.";

/// The semantic hit: about the same thing and shares almost no term with
/// the query, which is what the vector half is for.
const SEMANTIC: &str = "Reading source code into a syntax tree is done by walking the \
                        input once and deciding at each step which production applies, \
                        without ever revisiting an earlier choice.";

/// Neither: present so the rankings have something to rank below.
const UNRELATED: &str = "The kitchen inventory lists eleven jars of preserved lemons, \
                         four bags of dried chickpeas, and a single tin of anchovies.";

/// The query. Shares "parser" and "tokens" with LEXICAL and nothing much
/// with SEMANTIC.
const SEARCH: &str = "parser tokens";

/// A graph whose chunks carry real vectors from the real service, or `None`
/// with a named reason.
async fn fixtures(graph: &str) -> Option<(Client, Session)> {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket (set YEOMNA_TEST_DB)");
        return None;
    };
    let Some(sock) = embedder_socket() else {
        eprintln!(
            "SKIP: no embedder socket at the default path. Start the service: \
             cd services/embedder && uv run python server.py"
        );
        return None;
    };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_owner");
        return None;
    };
    let client = match EmbeddingClient::connect_at(&sock).await {
        Ok(c) if c.info().loaded => c,
        Ok(_) => {
            eprintln!("SKIP: the embedder is up and still loading its weights");
            return None;
        }
        Err(e) => {
            eprintln!("SKIP: the embedder at {sock} did not answer: {e}");
            return None;
        }
    };
    yeomna_store::apply_schema(&owner).await.expect("schema");
    owner
        .execute("DELETE FROM graphs WHERE name = $1", &[&graph])
        .await
        .unwrap();
    let g: i64 = owner
        .query_one(
            "INSERT INTO graphs (name) VALUES ($1) RETURNING id",
            &[&graph],
        )
        .await
        .unwrap()
        .get(0);
    let node: i64 = owner
        .query_one(
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, 'doc', 'document', '{\"title\":\"three paragraphs\"}'::jsonb)
             RETURNING id",
            &[&g],
        )
        .await
        .unwrap()
        .get(0);

    let info = client.info().clone();
    let _ = LIVE_MODEL.set((info.model.clone(), info.model_revision.clone()));
    for (i, text) in [LEXICAL, SEMANTIC, UNRELATED].into_iter().enumerate() {
        let chunk: i64 = owner
            .query_one(
                "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char)
                 VALUES ($1, $2, $3, 0, $4) RETURNING id",
                &[&node, &(i as i32), &text, &(text.len() as i32)],
            )
            .await
            .unwrap()
            .get(0);
        let vector = client
            .embed_one(text, "retrieval.passage")
            .await
            .expect("the service embeds a short passage");
        let literal = format!(
            "[{}]",
            vector
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        owner
            .execute(
                "INSERT INTO embeddings (chunk_id, vec, model, model_hash, model_revision, task)
                 VALUES ($1, $2::text::halfvec, $3, $4, $5, 'retrieval.passage')",
                &[
                    &chunk,
                    &literal,
                    &info.model,
                    &yeomna_keys::model_hash(&info.model),
                    &info.model_revision,
                ],
            )
            .await
            .unwrap();
    }

    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app connects");
    Some((
        owner,
        Session::new(app, "hybrid-test")
            .with_graph(graph)
            .with_embedder(sock),
    ))
}

/// The model and revision the live service serves, recorded by `fixtures`
/// so a test that rewrites a stored cohort can put the real one back.
static LIVE_MODEL: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

fn data(e: &Envelope) -> &Value {
    e.data.as_ref().expect("an envelope with data")
}

fn ask(hybrid: bool) -> Verb {
    Verb::Query(QueryRequest {
        search_text: SEARCH.into(),
        limit: 10,
        kind: None,
        hybrid,
        structural: false,
    })
}

/// FR1, FR2, FR3, EC-2. The claim the store PRD has carried since it was
/// drafted, made true: one statement, two rankings, fused.
#[tokio::test]
async fn fusion_finds_what_meaning_finds_and_prefers_agreement() {
    let Some((_owner, s)) = fixtures("hq_fuse").await else {
        return;
    };

    // Keyword alone finds the lexical chunk and not the semantic one. That
    // is the gap fusion exists to close, asserted rather than assumed.
    let keyword = data(&s.call(&ask(false)).await).clone();
    let keys: Vec<i64> = keyword["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["chunk_index"].as_i64().unwrap())
        .collect();
    assert!(
        keys.contains(&0),
        "keyword finds the lexical chunk: {keys:?}"
    );
    assert!(
        !keys.contains(&1),
        "and does not find the semantic one, which shares no term: {keys:?}"
    );

    let d = data(&s.call(&ask(true)).await).clone();
    assert_eq!(d["ranking"], "hybrid");
    assert_eq!(d["fusion"]["method"], "rrf");
    assert_eq!(d["fusion"]["k"], 60.0);

    // FR4: the query task was read from the corpus, not assumed.
    assert_eq!(d["cohort"]["corpus_task"], "retrieval.passage");
    assert_eq!(d["cohort"]["query_task"], "retrieval.query");
    assert_eq!(d["cohort"]["model_revision"].as_str().unwrap().len(), 40);

    let hits = d["hits"].as_array().unwrap();
    let by_index: std::collections::HashMap<i64, &Value> = hits
        .iter()
        .map(|h| (h["chunk_index"].as_i64().unwrap(), h))
        .collect();

    // FR3: the semantic chunk is here, and keyword alone did not find it.
    assert!(
        by_index.contains_key(&1),
        "fusion found the chunk that matches by meaning: {hits:?}"
    );

    // And the vector half ranked it *for the right reason*. Being present
    // is not enough: with three chunks and a candidate depth of fifty, the
    // vector CTE returns all three whatever the vector is, so every other
    // assertion here passes against a random query vector. Measured live,
    // the real distances are 0.397 for the lexical chunk, 0.480 for the
    // semantic one, and 0.685 for the unrelated one, so this ordering is
    // the claim that the vector half understood the query.
    let unrelated = by_index[&2];
    let semantic = by_index[&1];
    let sem_v = semantic["vector_rank"]
        .as_i64()
        .expect("the vector half saw it");
    let unr_v = unrelated["vector_rank"]
        .as_i64()
        .expect("and saw this one too");
    assert!(
        sem_v < unr_v,
        "the chunk about parsing outranks the one about preserved lemons, \
         which is the vector half doing its job rather than returning rows: \
         semantic at {sem_v}, unrelated at {unr_v}"
    );

    // EC-2: the lexical chunk appears once, carrying both ranks, and
    // outscores a chunk that only one source found.
    let lexical = by_index[&0];
    assert!(
        lexical["text_rank"].is_number() && lexical["vector_rank"].is_number(),
        "the chunk both sources found carries both ranks: {lexical}"
    );
    assert_eq!(
        hits.iter().filter(|h| h["chunk_index"] == json!(0)).count(),
        1,
        "and appears once rather than once per source"
    );
    assert!(
        lexical["rank"].as_f64().unwrap() > semantic["rank"].as_f64().unwrap(),
        "agreement between the two sources outscores one strong signal: {lexical} vs {semantic}"
    );

    // A chunk only one source found carries only that source's rank.
    assert!(
        semantic["text_rank"].is_null() || semantic["vector_rank"].is_null(),
        "one-source hits say which source: {semantic}"
    );
}

/// FR5 and EC-4. A graph with chunks and no vectors is what an
/// over-ceiling document leaves behind, and it refuses rather than quietly
/// ranking by keyword alone.
#[tokio::test]
async fn a_graph_with_no_vectors_refuses_rather_than_falling_back() {
    let Some((owner, s)) = fixtures("hq_novec").await else {
        return;
    };
    owner
        .execute(
            "DELETE FROM embeddings e USING chunks c, nodes n, graphs g
             WHERE e.chunk_id = c.id AND c.node_id = n.id AND n.graph_id = g.id
               AND g.name = 'hq_novec'",
            &[],
        )
        .await
        .unwrap();

    let env = s.call(&ask(true)).await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("not-found"), "{msg}");
    assert!(msg.contains("no embeddings"), "{msg}");
    assert!(
        msg.contains("embed: true"),
        "it says what would fix it: {msg}"
    );

    // And the chunks are still findable without the flag, so the refusal is
    // about the ranking mode rather than about the graph.
    let d = data(&s.call(&ask(false)).await).clone();
    assert!(!d["hits"].as_array().unwrap().is_empty(), "{d}");
}

/// FR6. Two cohorts in one graph and no single query vector comparable to
/// both, which is the situation R26's columns exist to make visible.
#[tokio::test]
async fn a_graph_with_two_cohorts_refuses_and_names_them() {
    let Some((owner, s)) = fixtures("hq_cohorts").await else {
        return;
    };
    // Move one row to another task, leaving the model and revision alone,
    // so the only thing making it a second cohort is the adapter.
    owner
        .execute(
            "UPDATE embeddings SET task = 'code'
             WHERE chunk_id = (
                 SELECT c.id FROM chunks c
                 JOIN nodes n ON n.id = c.node_id
                 JOIN graphs g ON g.id = n.graph_id
                 WHERE g.name = 'hq_cohorts' AND c.chunk_index = 2
             )",
            &[],
        )
        .await
        .unwrap();

    let env = s.call(&ask(true)).await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("invalid-args"), "{msg}");
    assert!(msg.contains("2 embedding cohorts"), "{msg}");
    assert!(
        msg.contains("retrieval.passage") && msg.contains("code"),
        "both are named: {msg}"
    );
    assert!(
        msg.contains("without hybrid"),
        "it says what would work now: {msg}"
    );
}

/// A cohort is a property of a graph, so an unscoped session cannot have
/// one. Across a database there could be as many cohorts as graphs.
#[tokio::test]
async fn hybrid_needs_a_graph() {
    let Some(dir) = socket_dir() else { return };
    let Some(sock) = embedder_socket() else {
        return;
    };
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        return;
    };
    let s = Session::new(app, "hybrid-unscoped").with_embedder(sock);
    let env = s.call(&ask(true)).await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("invalid-args"), "{msg}");
    assert!(msg.contains("needs a graph"), "{msg}");
}

/// FR7. No embedder configured, so no query vector, so no fusion, and the
/// message names the key rather than a hole.
#[tokio::test]
async fn hybrid_without_an_embedder_names_the_config_key() {
    let Some((_owner, _s)) = fixtures("hq_noembedder").await else {
        return;
    };
    // The same session without the embedder it was given.
    let dir = socket_dir().expect("fixtures proved it");
    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_app");
        return;
    };
    let bare = Session::new(app, "hybrid-no-embedder").with_graph("hq_noembedder");
    let env = bare.call(&ask(true)).await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("internal"), "{msg}");
    assert!(msg.contains("embedder_socket"), "{msg}");
    assert!(
        !msg.contains("H4"),
        "H4 is filled, so this is not a hole: {msg}"
    );
}

/// FR8. `structural` is not affected by any of this and still names H9.
#[tokio::test]
async fn structural_still_names_h9() {
    let Some((_owner, s)) = fixtures("hq_structural").await else {
        return;
    };
    let env = s
        .call(&Verb::Query(QueryRequest {
            search_text: SEARCH.into(),
            limit: 10,
            kind: None,
            hybrid: false,
            structural: true,
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("unimplemented") && msg.contains("H9"),
        "{msg}"
    );
}

/// EC-1 as the build settled it. A query matching nothing lexically still
/// returns the graph's nearest vectors, because a nearest search has no
/// distance threshold and always has a nearest.
///
/// The spec first said this should be an empty list. It cannot be, short of
/// a distance cutoff, and a cutoff is a retrieval-quality decision with a
/// number in it that M1 has not measured. What is asserted instead is the
/// part that matters: nonsense contributes nothing to the keyword half, so
/// every hit is the vector half's and says so.
#[tokio::test]
async fn a_query_matching_no_term_is_ranked_by_the_vector_half_alone() {
    let Some((_owner, s)) = fixtures("hq_empty").await else {
        return;
    };
    let d = data(
        &s.call(&Verb::Query(QueryRequest {
            search_text: "zzyzx qwertyuiop".into(),
            limit: 5,
            kind: None,
            hybrid: true,
            structural: false,
        }))
        .await,
    )
    .clone();
    let hits = d["hits"].as_array().unwrap();
    assert!(
        !hits.is_empty(),
        "a nearest search always has a nearest, so this is not empty: {d}"
    );
    for h in hits {
        assert!(
            h["text_rank"].is_null(),
            "no keyword match for nonsense: {h}"
        );
    }
}

/// FR9. `k` is not a field a caller can set, which the closed contract
/// enforces: `QueryRequest` denies unknown fields.
#[test]
fn the_fusion_constant_is_not_a_request_field() {
    for attempt in [
        r#"{"verb":"query","args":{"search_text":"x","hybrid":true,"k":1}}"#,
        r#"{"verb":"query","args":{"search_text":"x","hybrid":true,"rrf_k":1}}"#,
        r#"{"verb":"query","args":{"search_text":"x","hybrid":true,"vector":[0.1]}}"#,
        r#"{"verb":"query","args":{"search_text":"x","hybrid":true,"candidate_depth":5}}"#,
    ] {
        let e =
            serde_json::from_str::<Verb>(attempt).expect_err("the contract denies unknown fields");
        assert!(e.to_string().contains("unknown field"), "{e}");
    }
    // And a caller-supplied vector has no field to arrive in, which is
    // PRD-embedder D7: reaching vector search without `embed.text` would be
    // a side door around a verb.
}

/// `hnsw.ef_search` caps what an index scan returns, measured.
///
/// **This is not a property of the verb.** The fusion's vector half does
/// not reach the index at all, because the graph filter arrives through
/// `chunks -> nodes -> graphs` and the planner drives from the graph side
/// (M5). What this measures is pgvector itself, through a raw unfiltered
/// query with the planner forced onto the index, and it is here as evidence
/// for the fix M5 describes rather than as a claim about what `query
/// --hybrid` does today.
///
/// The measurement: an HNSW scan returns at most `hnsw.ef_search` rows,
/// which defaults to 40, so asking the index for 150 candidates returns 40
/// and says nothing about it. Whoever gives `embeddings` a filter column
/// needs `ef_search` and `iterative_scan` in the same change, or the
/// candidate depth becomes a number the index does not honor.
#[tokio::test]
async fn the_default_ef_search_caps_an_index_scan() {
    let Some(dir) = socket_dir() else { return };
    let Some(sock) = embedder_socket() else {
        return;
    };
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        return;
    };
    let vectors: i64 = owner
        .query_one("SELECT count(*) FROM embeddings", &[])
        .await
        .unwrap()
        .get(0);
    // The default ef_search is 40, so proving anything needs more rows than
    // that plus room above the ask.
    if vectors < 200 {
        eprintln!("SKIP: only {vectors} vectors on this cluster, too few to out-rank ef_search");
        return;
    }
    let client = match EmbeddingClient::connect_at(&sock).await {
        Ok(c) if c.info().loaded => c,
        _ => {
            eprintln!("SKIP: the embedder is not serving");
            return;
        }
    };
    let vector = client
        .embed_one("a query for the candidate depth", "retrieval.query")
        .await
        .expect("embeds");
    let literal = format!(
        "[{}]",
        vector
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );

    // Without the settings, on the index: capped at the default ef_search.
    owner
        .batch_execute("BEGIN; SET LOCAL enable_seqscan = off;")
        .await
        .unwrap();
    let capped: i64 = owner
        .query_one(
            "SELECT count(*) FROM (
                 SELECT chunk_id FROM embeddings
                 ORDER BY vec <=> $1::text::halfvec LIMIT 150
             ) t",
            &[&literal],
        )
        .await
        .unwrap()
        .get(0);
    owner.batch_execute("ROLLBACK").await.unwrap();

    // With them: the full ask.
    owner
        .batch_execute(
            "BEGIN; SET LOCAL enable_seqscan = off; SET LOCAL hnsw.ef_search = 150; \
             SET LOCAL hnsw.iterative_scan = strict_order;",
        )
        .await
        .unwrap();
    let honored: i64 = owner
        .query_one(
            "SELECT count(*) FROM (
                 SELECT chunk_id FROM embeddings
                 ORDER BY vec <=> $1::text::halfvec LIMIT 150
             ) t",
            &[&literal],
        )
        .await
        .unwrap()
        .get(0);
    owner.batch_execute("ROLLBACK").await.unwrap();

    assert!(
        capped < 150,
        "the default ef_search caps an index scan, which is the whole reason \
         the fusion sets it: got {capped} of 150"
    );
    assert_eq!(
        honored, 150,
        "and with ef_search at the candidate depth the index returns it"
    );
}

/// EC-6. An embedder that answers and has not loaded its weights, which is
/// what the first seconds after a start look like.
///
/// Served by a socket this test owns rather than by the real service, since
/// the real one is loaded by the time a suite reaches it and the branch
/// would otherwise be unreachable from the gate. Sixty lines of canned
/// HTTP is cheaper than an untested refusal.
#[tokio::test]
async fn an_embedder_still_loading_refuses_before_the_fusion() {
    let Some((_owner, _s)) = fixtures("hq_loading").await else {
        return;
    };
    let dir = socket_dir().expect("fixtures proved it");
    let temp = tempfile::TempDir::new().unwrap();
    let fake = temp.path().join("embedder.sock");

    // A service that answers /v1/info with loaded: false and nothing else.
    let listener = tokio::net::UnixListener::bind(&fake).expect("binds");
    let serving = tokio::spawn(async move {
        let body = serde_json::json!({
            "model": "jinaai/jina-embeddings-v4",
            "model_revision": "853c867b65b749f3c3c72a06868140d842e04f06",
            "dimension": 2048,
            "max_tokens": 16384,
            "tasks": ["retrieval.passage", "retrieval.query"],
            "device": "cuda:2",
            "loaded": false,
        })
        .to_string();
        // Two connections: one for the client's connect-time /v1/info, and
        // one spare so a retry does not hang the test.
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });

    let Ok(app) = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_app");
        return;
    };
    let s = Session::new(app, "hybrid-loading")
        .with_graph("hq_loading")
        .with_embedder(fake.display().to_string());
    let env = s.call(&ask(true)).await;
    serving.abort();

    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(msg.starts_with("internal"), "{msg}");
    assert!(
        msg.contains("loading its weights"),
        "it says what is happening rather than that something failed: {msg}"
    );
}

/// A corpus embedded by another model, or another snapshot of the same
/// model, is refused rather than ranked.
///
/// A cohort is three things and the first cut checked one of them. The
/// service is swappable and its weights are pinned by configuration, so
/// this is a state an operator reaches by restarting one unit: the corpus
/// keeps the model and revision it was written with, the embedder serves
/// whatever it now loads, and a query vector from the second geometry ranks
/// the first into plausible nonsense with nothing in the result saying so.
///
/// Driven by rewriting the stored cohort, which is the cheap direction: the
/// alternative is loading a second 7 GB model onto the card.
#[tokio::test]
async fn a_corpus_from_another_cohort_is_refused_rather_than_ranked() {
    let Some((owner, s)) = fixtures("hq_othermodel").await else {
        return;
    };

    for (column, value, wanted) in [
        ("model", "someone-else/embeddings-v9", "someone-else"),
        (
            "model_revision",
            "0000000000000000000000000000000000000000",
            "0000000",
        ),
    ] {
        owner
            .execute(
                &format!(
                    "UPDATE embeddings e SET {column} = $1
                     FROM chunks c, nodes n, graphs g
                     WHERE e.chunk_id = c.id AND c.node_id = n.id AND n.graph_id = g.id
                       AND g.name = 'hq_othermodel'"
                ),
                &[&value],
            )
            .await
            .unwrap();

        let env = s.call(&ask(true)).await;
        assert!(!env.success, "{column} mismatch must refuse");
        let msg = env.error.unwrap();
        assert!(msg.starts_with("internal"), "{msg}");
        assert!(
            msg.contains(wanted),
            "the refusal names what the corpus was embedded by: {msg}"
        );
        assert!(
            msg.contains("plausible nonsense"),
            "and says why it is not merely a mismatch: {msg}"
        );

        // Put it back, so the second pass tests its own column alone.
        owner
            .execute(
                "UPDATE embeddings e SET model = $1, model_revision = $2
                 FROM chunks c, nodes n, graphs g
                 WHERE e.chunk_id = c.id AND c.node_id = n.id AND n.graph_id = g.id
                   AND g.name = 'hq_othermodel'",
                &[&LIVE_MODEL.get().unwrap().0, &LIVE_MODEL.get().unwrap().1],
            )
            .await
            .unwrap();
    }

    // And with the cohort restored it answers, so the refusal is about the
    // mismatch rather than about the graph.
    let env = s.call(&ask(true)).await;
    assert!(env.success, "{:?}", env.error);
}
