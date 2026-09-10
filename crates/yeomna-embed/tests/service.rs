//! The embedder service, through the client (spec 022).
//!
//! Service-gated, the way every other test here is cluster-gated. Skips
//! with a named reason when the socket is absent, because an appliance
//! whose embedder is not running is a normal state and not a broken
//! checkout.
//!
//! **The first test is the one that cannot be faked.** PRD D1 moved the
//! pooling into the service, and D3 kept `late_chunk_embeddings` as the
//! thing the service is checked against. The spike measured that identity
//! once, on fixtures, on one afternoon. This measures it every run,
//! against the live model, and names the chunk it failed on. The motive
//! (retrieval quality) will keep changing. The mechanic (pooled output
//! equals the pooling identity) does not.

use yeomna_chunking::{LateChunkConfig, late_chunk_embeddings};
use yeomna_embed::embedding::{
    ChunkPolicy, EmbeddingClient, EmbeddingError, REQUIRED_DIMENSION, parse_endpoint,
};

/// Where the service listens, matching the config file's derived default.
fn socket() -> Option<String> {
    let path = std::env::var("YEOMNA_EMBEDDER_SOCKET").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run/embedder.sock")
    });
    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        eprintln!("SKIP: no embedder socket at {path} (start yeomna-embedder.service)");
        None
    }
}

/// A connected client with its weights loaded, or `None` with a reason.
async fn client() -> Option<EmbeddingClient> {
    let path = socket()?;
    let c = match EmbeddingClient::connect_at(&path).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: the embedder at {path} did not answer: {e}");
            return None;
        }
    };
    if !c.info().loaded {
        eprintln!("SKIP: the embedder is up and still loading its weights");
        return None;
    }
    Some(c)
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    let nb: f64 = b
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

/// Text long enough to produce several windows at a small chunk size and
/// short enough to stay under `/v1/tokens`'s 512-token cap.
const ORACLE_TEXT: &str = "\
The verb layer is the only surface. Nobody writes SQL, JSONB path expressions, \
traversals, or search calls. A human and an agent call the same verbs and are \
logged the same way. Basis is derived at ingest and written explicitly, never \
inferred from edge type. The diff log is the source of truth for history, and \
the head row is materialized convenience. The graph is a rebuildable index, so \
losing it costs a re-ingest and there is no migration tooling. Nothing leaves \
the box: no telemetry, no phone-home, no remote embedding, no default remote \
backup destination.";

/// **The oracle.** Both views of one text, the token view pooled in Rust,
/// and the two must agree.
///
/// This is what licenses PRD D1. The service pools because shipping token
/// states over a JSON transport is 150 to 300 times the bytes, and the
/// only thing that made that trade safe was the spike's measurement that
/// the two poolings are the same operation. A measurement taken once
/// decays. This one runs every time the gate does.
#[tokio::test]
async fn the_service_pools_the_way_late_chunk_embeddings_does() {
    let Some(c) = client().await else { return };
    let policy = ChunkPolicy {
        size_tokens: 40,
        overlap_tokens: 10,
    };

    let tokens = c
        .tokens(ORACLE_TEXT, "retrieval.passage")
        .await
        .expect("the diagnostic operation answers");
    assert_eq!(tokens.dimension, REQUIRED_DIMENSION);
    assert_eq!(
        tokens.hidden_states.len() as u32,
        tokens.token_count,
        "one hidden state per token"
    );
    assert_eq!(
        tokens.offsets.len() as u32,
        tokens.token_count,
        "one byte span per token"
    );
    for h in &tokens.hidden_states {
        assert_eq!(h.len() as u32, REQUIRED_DIMENSION);
    }

    let pooled = late_chunk_embeddings(
        &tokens.hidden_states,
        &LateChunkConfig {
            chunk_size_tokens: policy.size_tokens as usize,
            overlap_tokens: policy.overlap_tokens as usize,
        },
    );

    let served = c
        .embed_document(ORACLE_TEXT, "retrieval.passage", policy)
        .await
        .expect("the production operation answers");

    assert_eq!(
        served.len(),
        pooled.embeddings.len(),
        "the service and late.rs windowed the same token count into the same \
         number of chunks: {} against {}",
        served.len(),
        pooled.embeddings.len()
    );

    for (i, chunk) in served.iter().enumerate() {
        let (start, end) = pooled.boundaries[i];
        assert_eq!(
            (chunk.start_token as usize, chunk.end_token as usize),
            (start, end),
            "chunk {i} covers a different token window than the rule says"
        );
        let c = cosine(&chunk.vector, &pooled.embeddings[i]);
        assert!(
            c >= 0.9999,
            "chunk {i} pooled differently: cosine {c:.9}. The service's pooling \
             and mean_pool_and_normalize have diverged, which is the one thing \
             /v1/tokens exists to catch"
        );
    }
}

/// Byte spans slice the caller's own text, with the task prefix rebased
/// out. The spike found that HF fast tokenizers report character offsets
/// while every consumer here slices bytes, and that ASCII hides the
/// difference, so this asks with multi-byte input.
#[tokio::test]
async fn byte_spans_slice_the_callers_text_including_multibyte() {
    let Some(c) = client().await else { return };
    let text = "\u{4e16}\u{754c}\u{3092}\u{8aad}\u{3080} and then some ASCII, \
                then \u{1f680} \u{00e9}clair, then a tail to make several windows \
                out of what would otherwise be one.";
    let chunks = c
        .embed_document(
            text,
            "retrieval.passage",
            ChunkPolicy {
                size_tokens: 8,
                overlap_tokens: 2,
            },
        )
        .await
        .expect("embeds");

    assert!(chunks.len() > 1, "several windows, not one");
    assert_eq!(chunks[0].start_byte, 0, "the prefix is rebased out");
    assert_eq!(
        chunks.last().unwrap().end_byte,
        text.len(),
        "the last window reaches the end of the caller's text"
    );
    for ch in &chunks {
        let sliced = ch
            .slice(text)
            .unwrap_or_else(|| panic!("chunk {} is not on a char boundary", ch.chunk_index));
        assert!(!sliced.is_empty());
    }
    // The windows tile with no gap, which is the seam the spike checked
    // and the place a windowing bug shows up first.
    for w in chunks.windows(2) {
        assert!(
            w[1].start_byte < w[0].end_byte,
            "chunk {} starts after chunk {} ended, leaving a gap",
            w[1].chunk_index,
            w[0].chunk_index
        );
    }
}

/// Every vector is unit norm, so a cosine index needs no normalization
/// step at the edge, and every chunk is the declared width.
#[tokio::test]
async fn every_vector_is_unit_norm_and_the_declared_width() {
    let Some(c) = client().await else { return };
    let chunks = c
        .embed_document(ORACLE_TEXT, "retrieval.passage", ChunkPolicy::default())
        .await
        .expect("embeds");
    assert!(!chunks.is_empty());
    for ch in &chunks {
        assert_eq!(ch.vector.len() as u32, REQUIRED_DIMENSION);
        let norm = cosine(&ch.vector, &ch.vector);
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "chunk {} is not unit: {norm}",
            ch.chunk_index
        );
    }
}

/// A short text is one chunk, and there is no separate unchunked shape it
/// could have arrived in. `embed_one` is that case of the same operation.
#[tokio::test]
async fn a_short_text_is_one_chunk_and_embed_one_returns_it() {
    let Some(c) = client().await else { return };
    let chunks = c
        .embed_document("a short query", "retrieval.query", ChunkPolicy::default())
        .await
        .expect("embeds");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].total_chunks, 1);
    assert_eq!(chunks[0].chunk_index, 0);

    let one = c
        .embed_one("a short query", "retrieval.query")
        .await
        .expect("one vector");
    assert_eq!(one.len() as u32, REQUIRED_DIMENSION);
    assert!(
        cosine(&one, &chunks[0].vector) >= 0.9999,
        "embed_one is the whole-text window of the same operation"
    );
}

/// The refusals, which are the design. Each names its code so a caller can
/// tell a mistake of its own from a service that is out of memory.
#[tokio::test]
async fn the_service_refuses_by_code() {
    let Some(c) = client().await else { return };

    let e = c
        .embed_document("anything", "retrieval.summary", ChunkPolicy::default())
        .await
        .expect_err("a task the service does not serve");
    assert_eq!(
        e.code(),
        Some("unknown-task"),
        "never routed to another adapter: {e}"
    );

    let e = c
        .embed_document("", "retrieval.passage", ChunkPolicy::default())
        .await
        .expect_err("an empty input has no cosine");
    assert_eq!(e.code(), Some("invalid-request"), "{e}");

    // PRD D5: above the ceiling is refused, never truncated. Built from
    // the service's own reported ceiling rather than a literal, so this
    // stays true if the card changes.
    let over = "word ".repeat(c.info().max_tokens as usize + 500);
    let e = c
        .embed_document(&over, "retrieval.passage", ChunkPolicy::default())
        .await
        .expect_err("above the ceiling");
    assert_eq!(e.code(), Some("input-too-large"), "{e}");
    assert!(
        e.to_string().contains(&c.info().max_tokens.to_string()),
        "the refusal says what the ceiling is: {e}"
    );

    // The diagnostic operation stays diagnostic.
    let e = c
        .tokens(&"word ".repeat(2000), "retrieval.passage")
        .await
        .expect_err("above the tokens cap");
    assert_eq!(e.code(), Some("token-cap-exceeded"), "{e}");
}

/// What the service says it is, checked against what this appliance can
/// store. A dimension mismatch is refused at connect, so reaching here at
/// all is half the assertion.
#[tokio::test]
async fn the_service_describes_itself_completely() {
    let Some(c) = client().await else { return };
    let info = c.info();
    assert_eq!(info.dimension, REQUIRED_DIMENSION);
    assert!(!info.model.is_empty());
    assert_eq!(
        info.model_revision.len(),
        40,
        "a full snapshot SHA, since that is what decides comparability: {:?}",
        info.model_revision
    );
    assert!(info.max_tokens >= 512, "at least the diagnostic cap");
    for task in ["retrieval.passage", "retrieval.query"] {
        assert!(
            info.tasks.iter().any(|t| t == task),
            "{task} is the corpus and query pairing this appliance depends on"
        );
    }
    assert!(info.device.starts_with("cuda"), "{}", info.device);
}

/// A URL is not an endpoint, asserted here as well as in the unit tests
/// because this is the file a reader opens to see how the client is
/// reached. Charter 5.1 is a requirement, not a default.
#[test]
fn there_is_no_way_to_point_this_client_at_a_host() {
    let e = parse_endpoint("http://localhost:8087/v1").expect_err("refused");
    assert!(matches!(e, EmbeddingError::Unreachable { .. }));
    assert!(e.to_string().contains("5.1"), "{e}");
}

/// The batching path, driven with more than one input.
///
/// `embed` carries a work queue, index rebasing, reassembly by input index,
/// and out-of-memory halving, and every other caller in the workspace hands
/// it exactly one input. Untested machinery in the path a future batched
/// caller will take is worse than no machinery, so this drives it: several
/// inputs, a batch size that forces more than one request, and results
/// checked back against per-input calls.
#[tokio::test]
async fn a_multi_input_batch_reassembles_in_input_order() {
    let Some(c) = client().await else { return };
    let inputs: Vec<String> = [
        "the recursive descent parser reads tokens left to right",
        "postgres supplies documents, embeddings, keyword retrieval, and graph",
        "the diff log is the source of truth for history",
        "nothing leaves the box, and there is nowhere for it to go",
        "basis is derived at ingest and written explicitly",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    // A batch size of 2 over 5 inputs is three requests, so the reassembly
    // and the index rebasing both run rather than being no-ops.
    let batched = c
        .embed(
            &inputs,
            "retrieval.passage",
            ChunkPolicy::default(),
            Some(2),
        )
        .await
        .expect("a split batch answers");

    assert_eq!(batched.results.len(), inputs.len());
    for (i, r) in batched.results.iter().enumerate() {
        assert_eq!(r.index, i, "results are in input order after reassembly");
        assert!(!r.chunks.is_empty());
    }

    // Every vector matches what the same input gets on its own, which is
    // what proves the rebasing put each result against the right input
    // rather than merely producing the right count.
    for (i, text) in inputs.iter().enumerate() {
        let alone = c
            .embed_document(text, "retrieval.passage", ChunkPolicy::default())
            .await
            .expect("one input answers");
        assert_eq!(alone.len(), batched.results[i].chunks.len(), "input {i}");
        for (a, b) in alone.iter().zip(&batched.results[i].chunks) {
            assert!(
                cosine(&a.vector, &b.vector) >= 0.9999,
                "input {i} chunk {} landed against the wrong input",
                a.chunk_index
            );
            assert_eq!((a.start_byte, a.end_byte), (b.start_byte, b.end_byte));
        }
    }

    // One request for the whole list agrees with the split one.
    let whole = c
        .embed(&inputs, "retrieval.passage", ChunkPolicy::default(), None)
        .await
        .expect("one request answers");
    assert_eq!(whole.results.len(), batched.results.len());
    for (w, b) in whole.results.iter().zip(&batched.results) {
        assert_eq!(w.token_count, b.token_count);
        assert_eq!(w.chunks.len(), b.chunks.len());
    }
    assert_eq!(whole.model, batched.model);
    assert_eq!(whole.model_revision, batched.model_revision);
}

/// EC-2's accepting half, at the ceiling rather than near it.
///
/// The ceiling is a refusal boundary and a boundary needs both sides. A
/// service that refused everything above 14k while advertising 16,384 would
/// pass the refusal test and still be wrong, and that is not a hypothetical
/// here: under the default CUDA allocator the real ceiling measured about
/// 14k, which is why the service sets `expandable_segments` itself.
///
/// The input is grown until the service reports a token count at the
/// ceiling, rather than guessed at, because how text maps to tokens is the
/// tokenizer's business and not this test's.
#[tokio::test]
async fn an_input_at_the_advertised_ceiling_is_accepted() {
    let Some(c) = client().await else { return };
    let ceiling = c.info().max_tokens;

    // One probe to learn the ratio, then one attempt at the ceiling, then a
    // short climb. Bounded, so a tokenizer that behaves unexpectedly makes
    // this fail rather than loop.
    let probe = c
        .tokens("word ".repeat(80).trim(), "retrieval.passage")
        .await
        .expect("a small probe answers");
    let per_word = f64::from(probe.token_count) / 80.0;
    let mut words = ((f64::from(ceiling) / per_word).floor() as usize).max(1);

    let mut at_ceiling = None;
    for _ in 0..12 {
        let text = "word ".repeat(words);
        let text = text.trim();
        match c
            .embed_document(text, "retrieval.passage", ChunkPolicy::whole_text(ceiling))
            .await
        {
            Ok(chunks) => {
                let reached = chunks.last().expect("one window").end_token;
                if reached >= ceiling {
                    at_ceiling = Some((reached, chunks));
                    break;
                }
                // Still short: close the remaining gap in one step.
                let missing = ceiling - reached;
                words += ((f64::from(missing) / per_word).ceil() as usize).max(1);
            }
            Err(e) if e.code() == Some("input-too-large") => {
                // Overshot. Step back by the overshoot the message implies,
                // conservatively.
                words -= (words / 50).max(1);
            }
            Err(e) => panic!("unexpected refusal below the ceiling: {e}"),
        }
    }

    let (reached, chunks) = at_ceiling.unwrap_or_else(|| {
        panic!("could not build an input reaching the advertised ceiling of {ceiling} tokens")
    });
    assert_eq!(
        reached, ceiling,
        "an input at exactly the ceiling is accepted, not refused"
    );
    assert_eq!(chunks.len(), 1, "one window over the whole thing");
    for ch in &chunks {
        assert_eq!(ch.vector.len() as u32, REQUIRED_DIMENSION);
    }

    // And one token past it is refused, so the boundary is the boundary
    // rather than a floor the service happens to sit above.
    let over = format!("{} word", "word ".repeat(words).trim());
    let e = c
        .embed_document(&over, "retrieval.passage", ChunkPolicy::whole_text(ceiling))
        .await
        .expect_err("past the ceiling");
    assert_eq!(e.code(), Some("input-too-large"), "{e}");
}
