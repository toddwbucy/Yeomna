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

use std::path::PathBuf;
use std::time::Duration;

use yeomna_embed::embedding::{EmbeddingClientConfig, EmbeddingEndpoint};
use yeomna_embed::extraction::{ExtractOptions, ExtractionClientConfig, ExtractionEndpoint};

#[test]
fn embedding_defaults_are_the_shipped_values() {
    let c = EmbeddingClientConfig::default();
    // Port 8087 is deliberate: it dodges vLLM and uvicorn on 8000 and the
    // weaver-serve LLM API on 8080. Moving it is a deployment decision,
    // not a tidy-up.
    let EmbeddingEndpoint::Tcp(url) = c.endpoint else {
        panic!("the default endpoint is TCP, since the Unix seam is opt-in");
    };
    assert_eq!(url, "http://localhost:8087/v1");
    // The bound model, matching the golden model_hash inputs in
    // yeomna-keys and the vectors the store's halfvec(2048) column holds.
    assert_eq!(c.model, "jinaai/jina-embeddings-v4");
    assert_eq!(
        c.timeout,
        Duration::from_secs(300),
        "5 min for large batches"
    );
    assert_eq!(c.connect_timeout, Duration::from_secs(10));
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
