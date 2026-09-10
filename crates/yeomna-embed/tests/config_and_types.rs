//! The config and type portions of the embed client integration tests,
//! deferred at the spec 003 lift and recorded in holes-ledger H11.
//!
//! These are the parts that need no live service: the shipped defaults and
//! the option constructors, exercised through the public API exactly as a
//! caller sees them.
//!
//! **Why the defaults are pinned rather than left to the code.** R3 was
//! ruled on 2026-08-14: the appliance ships a config file, and H10 will
//! move these values into it. Until then they are the interim surface, and
//! a config layer that silently changed one during that move would be a
//! behavior change wearing a refactor's clothes. These tests are what the
//! migration has to keep passing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use yeomna_embed::embedding::{
    ChunkPolicy, EmbeddingClientConfig, EmbeddingError, REQUIRED_DIMENSION, parse_endpoint,
};
use yeomna_embed::extraction::{ExtractOptions, ExtractionClientConfig, ExtractionEndpoint};

/// This test used to pin the TCP default at `http://localhost:8087/v1`.
/// It now pins its absence, which is the whole of PRD-embedder D10.
///
/// Charter 5.1: "Embedding is local. The embedder is GPU-resident on the
/// same machine, so document text is never transmitted to an embedding
/// API. This is a requirement, not a configuration default." Moving the
/// default to Unix would have left a key an operator could point
/// anywhere, and a requirement a config key can turn off is a
/// preference. The variant is gone instead, so the refusal below is a
/// property of the type rather than of a default.
#[test]
fn the_embedder_endpoint_cannot_be_a_url() {
    for url in [
        "http://localhost:8087/v1",
        "https://api.openai.com/v1",
        "http://192.168.0.203:8087",
    ] {
        let e = parse_endpoint(url).expect_err("a URL is not an endpoint");
        let said = e.to_string();
        assert!(said.contains("Unix socket"), "{said}");
        assert!(
            said.contains("5.1"),
            "the refusal names why, not just that: {said}"
        );
    }
}

#[test]
fn embedding_config_points_at_a_socket_and_keeps_the_lift_timeout() {
    let c = EmbeddingClientConfig::at("/run/yeomna/embedder.sock");
    assert_eq!(c.endpoint.path(), Path::new("/run/yeomna/embedder.sock"));
    assert_eq!(
        c.timeout,
        Duration::from_secs(300),
        "5 min, since a whole-document forward pass on one card is slow"
    );
    assert_eq!(
        parse_endpoint("unix:///run/yeomna/embedder.sock")
            .unwrap()
            .path(),
        c.endpoint.path(),
        "both spellings name the same socket"
    );
}

/// The refusal has to be reachable from a config file, not only from a
/// test.
///
/// The first cut handed the config string straight to `PathBuf`, so
/// `parse_endpoint` had no production caller: a URL became a literal
/// relative path and the operator got "no socket there" instead of the
/// reason there could not be one. Worse, `unix:///run/...`, the spelling
/// this crate's own documentation and the contract both use, became a
/// relative path with `unix:` in it. `from_config` is the seam the config
/// takes now.
#[test]
fn a_config_value_goes_through_the_validator_that_explains_it() {
    let e = EmbeddingClientConfig::from_config("http://localhost:8087/v1")
        .expect_err("a URL is refused, and the config is where that matters");
    assert!(matches!(e, EmbeddingError::Unreachable { .. }));
    assert!(e.to_string().contains("5.1"), "{e}");

    for spelling in [
        "unix:///run/yeomna/embedder.sock",
        "/run/yeomna/embedder.sock",
    ] {
        let c = EmbeddingClientConfig::from_config(spelling).expect("both spellings resolve");
        assert_eq!(
            c.endpoint.path(),
            Path::new("/run/yeomna/embedder.sock"),
            "{spelling} must reach the socket, not a path containing the scheme"
        );
    }
}

/// The dimension is not a preference. `embeddings.vec` is
/// `halfvec(2048)` because `vector(2048)` was measured on this cluster to
/// refuse an HNSW index, so a service serving another width has nowhere
/// to write and is refused at connect rather than after an ingest.
#[test]
fn the_required_dimension_matches_the_store_column() {
    assert_eq!(REQUIRED_DIMENSION, 2048);
}

/// The reference's chunking defaults, which the spike measured at 29
/// chunks tiling 8,631 tokens with no gap.
#[test]
fn the_chunk_policy_defaults_are_the_measured_ones() {
    let p = ChunkPolicy::default();
    assert_eq!(p.size_tokens, 500);
    assert_eq!(p.overlap_tokens, 200);
    // The whole-text policy is what makes `embed_one` the single-window
    // case of `embed` rather than a second response shape.
    let w = ChunkPolicy::whole_text(16384);
    assert_eq!(w.size_tokens, 16384);
    assert_eq!(w.overlap_tokens, 0);
}

#[test]
fn extraction_defaults_are_the_shipped_values() {
    let c = ExtractionClientConfig::default();
    // Unix by default here, unlike the embedder: the extractor is an
    // appliance-local service and the socket is its native seam.
    let ExtractionEndpoint::Unix(path) = c.endpoint else {
        panic!("the default extraction endpoint is a Unix socket");
    };
    assert_eq!(path, PathBuf::from("/run/yeomna/extractor.sock"));
    assert_eq!(
        c.timeout,
        Duration::from_secs(600),
        "10 min, since extraction of a large PDF is slow"
    );
    assert_eq!(c.connect_timeout, Duration::from_secs(10));
}

#[test]
fn extract_options_all_leaves_ocr_off() {
    let o = ExtractOptions::all();
    assert!(o.extract_tables);
    assert!(o.extract_equations);
    assert!(o.extract_images);
    // The load-bearing one. `all()` means all content types, not all
    // features: OCR is expensive enough that enabling it is always an
    // explicit act. A change here silently slows every ingest.
    assert!(!o.use_ocr, "OCR stays opt-in even under all()");
    assert!(o.source_type.is_none(), "auto-detect");
}

#[test]
fn extract_options_default_extracts_nothing_extra() {
    // Derived Default, so every flag is false. Distinct from `all()`, and
    // the distinction is easy to lose in a refactor.
    let o = ExtractOptions::default();
    assert!(!o.extract_tables);
    assert!(!o.extract_equations);
    assert!(!o.extract_images);
    assert!(!o.use_ocr);
    assert!(o.source_type.is_none());
}

/// Does a line reach for the ambient environment.
fn reads_environment(line: &str) -> bool {
    ["env::var", "env::vars", "env!("]
        .iter()
        .any(|p| line.contains(p))
}

#[test]
fn the_crate_consults_no_environment() {
    // R3's ruling in test form: configuration arrives through the config
    // type, never through the ambient environment. Asserted over the
    // source rather than by setting variables at runtime, for two
    // reasons. It covers every variable name instead of the two a test
    // author happened to imagine, and `set_var` is unsafe in this edition
    // because it races any concurrent `getenv`, which a threaded test
    // harness supplies for free.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut visited = 0;
    let mut stack = vec![src];
    while let Some(d) = stack.pop() {
        for f in std::fs::read_dir(&d).unwrap() {
            let p = f.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                visited += 1;
                for (n, line) in std::fs::read_to_string(&p).unwrap().lines().enumerate() {
                    if reads_environment(line) {
                        offenders.push(format!("{}:{}: {}", p.display(), n + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(
        visited > 0,
        "scanned nothing, so a clean result means nothing"
    );
    assert!(
        offenders.is_empty(),
        "yeomna-embed must take its configuration from its config types:\n{}",
        offenders.join("\n")
    );
}

/// The scan's negative test: a planted read is caught, so a clean result
/// means detection worked rather than detection broke.
#[test]
fn the_environment_scan_catches_a_planted_read() {
    assert!(reads_environment(
        r#"let x = std::env::var("YEOMNA_EMBED_ENDPOINT");"#
    ));
    assert!(reads_environment(r#"let x = env!("SOMETHING");"#));
    assert!(!reads_environment(
        "let endpoint = config.endpoint.clone();"
    ));
}
