//! The extraction seam (spec 015, R19).
//!
//! `Extractor` is the trait the orchestrator and the document ingest speak.
//! Two implementors: the gRPC client to the extraction service, unchanged
//! in behavior, and `NativeExtractor`, the docling converter crate running
//! in process for declarative formats. The native path handles what needs
//! no model and no download, refuses the rest per file, and never fails a
//! batch: one unconvertible file is a counted refusal (FR5).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use yeomna_embed::extraction::{ExtractOptions, ExtractResult, ExtractionClient, ExtractionError};
use yeomna_proto::extraction::SourceType;

/// Why a file did not extract. Per file and typed, so the operation can
/// count it and carry on.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// The extraction service refused or failed. Boxed because the
    /// transport error inside is large and every other variant is a
    /// path and a reason.
    #[error(transparent)]
    Service(Box<ExtractionError>),
    /// The backend does not handle this file's format.
    #[error("{path}: unsupported format: {reason}")]
    Unsupported { path: String, reason: String },
    /// The file claims a format and does not honor it.
    #[error("{path}: malformed: {reason}")]
    Malformed { path: String, reason: String },
    /// Converted cleanly to nothing worth indexing.
    #[error("{path}: no indexable content")]
    Empty { path: String },
    /// The converter itself failed, which is the backend's fault and
    /// not the file's, and an operator must be able to tell the two
    /// apart in the refusal list.
    #[error("{path}: converter failed: {reason}")]
    Backend { path: String, reason: String },
    /// The file could not be read.
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl From<ExtractionError> for ExtractError {
    fn from(e: ExtractionError) -> Self {
        Self::Service(Box::new(e))
    }
}

/// One document in, its extracted content out.
pub trait Extractor: Send + Sync {
    /// Extract a file's content. The options are the service contract's
    /// and a backend may ignore what does not apply to it.
    fn extract_file(
        &self,
        path: &Path,
        options: ExtractOptions,
    ) -> impl std::future::Future<Output = Result<ExtractResult, ExtractError>> + Send;
}

/// The socket client, by delegation. Behavior unchanged (FR1).
impl Extractor for ExtractionClient {
    async fn extract_file(
        &self,
        path: &Path,
        options: ExtractOptions,
    ) -> Result<ExtractResult, ExtractError> {
        Ok(ExtractionClient::extract_file(self, path, options).await?)
    }
}

/// The docling converter in process, for declarative formats.
///
/// The set of extensions it accepts is fixed at Markdown for spec 015.
/// Widening it to the office formats is a later spec's call, and until
/// then a DOCX that reaches this backend is an `Unsupported` refusal
/// rather than a surprise.
#[derive(Debug, Clone)]
pub struct NativeExtractor {
    extensions: BTreeSet<String>,
}

impl Default for NativeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeExtractor {
    /// Markdown only.
    pub fn new() -> Self {
        Self {
            extensions: ["md", "markdown"].into_iter().map(String::from).collect(),
        }
    }

    /// Does this backend take the file, judged by extension alone. What
    /// the bytes turn out to be is `extract_file`'s problem.
    pub fn handles(&self, path: &Path) -> bool {
        extension_of(path).is_some_and(|e| self.extensions.contains(&e))
    }
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

impl Extractor for NativeExtractor {
    async fn extract_file(
        &self,
        path: &Path,
        _options: ExtractOptions,
    ) -> Result<ExtractResult, ExtractError> {
        let shown = path.display().to_string();
        if !self.handles(path) {
            return Err(ExtractError::Unsupported {
                path: shown,
                reason: format!(
                    "extension {:?} is outside the native set",
                    extension_of(path).unwrap_or_default()
                ),
            });
        }
        // The converter is synchronous. Declarative conversion is
        // milliseconds, but it still does not belong on the executor.
        let owned = path.to_path_buf();
        tokio::task::spawn_blocking(move || convert_markdown(&owned))
            .await
            .map_err(|e| ExtractError::Backend {
                path: shown,
                reason: format!("conversion task failed: {e}"),
            })?
    }
}

/// The Markdown path: bytes checked, converted through docling, the
/// outline read from the source so the payload can say what the document
/// is about without anyone re-parsing it.
fn convert_markdown(path: &Path) -> Result<ExtractResult, ExtractError> {
    let shown = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|source| ExtractError::Io {
        path: shown.clone(),
        source,
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|e| ExtractError::Malformed {
        path: shown.clone(),
        reason: format!("not UTF-8 at byte {}", e.valid_up_to()),
    })?;
    let source = docling::SourceDocument::from_file(path).map_err(|e| ExtractError::Malformed {
        path: shown.clone(),
        reason: e.to_string(),
    })?;
    let converted = docling::DocumentConverter::new()
        .convert(source)
        .map_err(|e| ExtractError::Malformed {
            path: shown.clone(),
            reason: e.to_string(),
        })?;
    let full_text = converted.document.export_to_markdown();
    if full_text.trim().is_empty() {
        return Err(ExtractError::Empty { path: shown });
    }
    let outline = MarkdownOutline::of(text, path);
    let mut metadata = HashMap::new();
    metadata.insert("extractor".to_string(), "docling-native".to_string());
    metadata.insert("title".to_string(), outline.title);
    metadata.insert(
        "headings".to_string(),
        serde_json::to_string(&outline.headings).unwrap_or_else(|_| "[]".to_string()),
    );
    metadata.insert(
        "heading_count".to_string(),
        outline.headings.len().to_string(),
    );
    Ok(ExtractResult {
        full_text,
        tables: Vec::new(),
        equations: Vec::new(),
        images: Vec::new(),
        metadata,
        source_type: SourceType::Markdown,
    })
}

/// What a Markdown document is about, read from its headings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownOutline {
    /// The first level-one heading, or the file stem when there is none
    /// (EC-1).
    pub title: String,
    /// Every heading in order, as (level, text).
    pub headings: Vec<(u8, String)>,
}

impl MarkdownOutline {
    /// Headings inside fenced code are code, not structure, so fences
    /// are skipped.
    pub fn of(text: &str, path: &Path) -> Self {
        let mut headings = Vec::new();
        let mut in_fence = false;
        for raw in text.lines() {
            let line = raw.trim_end();
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let hashes = line.bytes().take_while(|b| *b == b'#').count();
            if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
                let title = line[hashes..].trim().to_string();
                if !title.is_empty() {
                    headings.push((hashes as u8, title));
                }
            }
        }
        let title = headings
            .iter()
            .find(|(level, _)| *level == 1)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("untitled")
                    .to_string()
            });
        Self { title, headings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[tokio::test]
    async fn markdown_converts_with_its_outline() {
        let (_d, p) = tmp(
            "spec.md",
            b"# The Spec\n\nSome prose.\n\n## Scope\n\n```rust\n# not a heading\n```\n\n### Detail\n",
        );
        let r = NativeExtractor::new()
            .extract_file(&p, ExtractOptions::all())
            .await
            .unwrap();
        assert!(r.full_text.contains("Some prose"));
        assert_eq!(r.metadata["title"], "The Spec");
        assert_eq!(r.metadata["heading_count"], "3", "the fenced hash is code");
        assert_eq!(r.metadata["extractor"], "docling-native");
    }

    #[tokio::test]
    async fn a_headingless_file_titles_itself_by_stem() {
        let (_d, p) = tmp("notes.md", b"just words\n");
        let r = NativeExtractor::new()
            .extract_file(&p, ExtractOptions::all())
            .await
            .unwrap();
        assert_eq!(r.metadata["title"], "notes");
        assert_eq!(r.metadata["heading_count"], "0");
    }

    #[tokio::test]
    async fn refusals_are_typed_per_file() {
        let x = NativeExtractor::new();
        let (_d, txt) = tmp("a.docx", b"whatever");
        assert!(matches!(
            x.extract_file(&txt, ExtractOptions::all()).await,
            Err(ExtractError::Unsupported { .. })
        ));
        let (_d, bad) = tmp("b.md", &[0x23, 0x20, 0xff, 0xfe, 0x0a]);
        assert!(matches!(
            x.extract_file(&bad, ExtractOptions::all()).await,
            Err(ExtractError::Malformed { .. })
        ));
        let (_d, empty) = tmp("c.md", b"\n\n");
        assert!(matches!(
            x.extract_file(&empty, ExtractOptions::all()).await,
            Err(ExtractError::Empty { .. })
        ));
        let missing = std::path::Path::new("/nonexistent/dir/d.md");
        assert!(matches!(
            x.extract_file(missing, ExtractOptions::all()).await,
            Err(ExtractError::Io { .. })
        ));
    }
}
