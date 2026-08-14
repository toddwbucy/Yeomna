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

#[test]
fn the_crate_consults_no_environment_for_its_endpoints() {
    // R3's ruling in test form: configuration arrives through the config
    // type, never through the ambient environment. If a future change
    // reaches for an env var, the defaults stop being the defaults and
    // this test is where that shows up.
    unsafe {
        std::env::set_var("YEOMNA_EMBED_ENDPOINT", "http://evil:1/v1");
        std::env::set_var("YEOMNA_EXTRACTOR_SOCKET", "/tmp/evil.sock");
    }
    let e = EmbeddingClientConfig::default();
    let x = ExtractionClientConfig::default();
    unsafe {
        std::env::remove_var("YEOMNA_EMBED_ENDPOINT");
        std::env::remove_var("YEOMNA_EXTRACTOR_SOCKET");
    }
    let EmbeddingEndpoint::Tcp(url) = e.endpoint else {
        panic!("unchanged by the environment");
    };
    assert_eq!(url, "http://localhost:8087/v1");
    let ExtractionEndpoint::Unix(path) = x.endpoint else {
        panic!("unchanged by the environment");
    };
    assert_eq!(path, PathBuf::from("/run/yeomna/extractor.sock"));
}
