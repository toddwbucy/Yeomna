//! The embedding seam (spec 022, H4).
//!
//! `Embedder` is the trait both ingest paths speak. Two implementors: the
//! socket client to the in-box embedder service, and [`HashEmbedder`], a
//! deterministic double.
//!
//! The double is the reason most of this crate's embedding tests need no
//! GPU. It was recorded as a harvest pointer in the holes ledger for
//! exactly this purpose, read from docling-rag and not adopted from it.
//! It reproduces the windowing rule and the byte-span contract faithfully
//! and invents the vectors, so every test of the wiring, the provenance
//! stamp, the count check, the spans, and the store write runs anywhere.
//! Only a test whose subject is the model needs the service.
//!
//! Its model name says what it is. A vector from the double must never be
//! mistakable for a real one in a store, because the store has no
//! re-embed-in-place tool to undo the confusion with (T4).

use std::future::Future;

use yeomna_chunking::TextChunk;

use sha2::{Digest, Sha256};
use yeomna_embed::embedding::{
    ChunkPolicy, EmbeddedChunk, EmbeddingClient, EmbeddingError, REQUIRED_DIMENSION,
};

/// What produced a vector.
///
/// Written onto the embedding document at ingest and from there into
/// `embeddings.model` and `embeddings.model_revision`, so a reader can tell
/// whether two vectors are comparable (R26). The revision is carried
/// because `model` and `model_hash` identify the weights by name and a
/// different snapshot of the same name is a different geometry.
///
/// It rides the document rather than being read back out of the parent
/// node's payload, which is what the store used to do. That made a
/// vector's provenance depend on a sibling row written by an earlier call,
/// and the codebase path never wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedderIdentity {
    pub model: String,
    pub model_revision: String,
    pub dimension: u32,
}

/// One document in, its chunk vectors out.
///
/// There is no per-chunk operation on purpose. Late chunking encodes the
/// whole document in one pass and decides afterwards where the chunks
/// were, and a trait method that took a chunk would invite the caller to
/// chunk first, which is the thing the reference did while its contract
/// promised otherwise.
pub trait Embedder: Send + Sync {
    /// What this embedder is, for the provenance stamp.
    fn identity(&self) -> EmbedderIdentity;

    /// The most tokens one pass accepts. A document above it is refused,
    /// never truncated (PRD D5).
    fn max_tokens(&self) -> u32;

    /// Encode a document and return its chunks, each carrying both a
    /// token range and a byte span into `text`.
    fn embed_document(
        &self,
        text: &str,
        task: &str,
        chunking: ChunkPolicy,
    ) -> impl Future<Output = Result<Vec<EmbeddedChunk>, EmbeddingError>> + Send;
}

/// One vector for one text, pooled over the whole thing.
///
/// A free function rather than a trait method, so the trait keeps one
/// operation and every implementor gets this for free with no way to
/// implement it inconsistently with the other.
pub async fn embed_one<E: Embedder>(
    embedder: &E,
    text: &str,
    task: &str,
) -> Result<Vec<f32>, EmbeddingError> {
    let chunks = embedder
        .embed_document(text, task, ChunkPolicy::whole_text(embedder.max_tokens()))
        .await?;
    let mut it = chunks.into_iter();
    let first = it
        .next()
        .ok_or_else(|| EmbeddingError::InvalidResponse("no chunk for the text".into()))?;
    if it.next().is_some() {
        return Err(EmbeddingError::InvalidResponse(
            "one window over the whole text produced more than one chunk".into(),
        ));
    }
    Ok(first.vector)
}

impl Embedder for EmbeddingClient {
    fn identity(&self) -> EmbedderIdentity {
        let info = self.info();
        EmbedderIdentity {
            model: info.model.clone(),
            model_revision: info.model_revision.clone(),
            dimension: info.dimension,
        }
    }

    fn max_tokens(&self) -> u32 {
        self.info().max_tokens
    }

    async fn embed_document(
        &self,
        text: &str,
        task: &str,
        chunking: ChunkPolicy,
    ) -> Result<Vec<EmbeddedChunk>, EmbeddingError> {
        EmbeddingClient::embed_document(self, text, task, chunking).await
    }
}

/// The model name the double reports. Deliberately not a real model, so a
/// store holding these vectors says so in every row.
pub const HASH_EMBEDDER_MODEL: &str = "yeomna-test/hash-embedder";

/// A deterministic embedder for tests.
///
/// Its vectors are derived from the chunk text by hash, so they are
/// stable across runs, machines, and architectures, and two identical
/// texts embed identically while two different ones almost never do. That
/// is enough to test everything about embedding except the embedding.
///
/// Its tokens are whitespace-separated runs. That is not the model's
/// tokenization and does not pretend to be. What matters is that it
/// windows by the same rule and reports byte spans that slice the input,
/// because those are the contracts the rest of the code depends on.
#[derive(Debug, Clone, Default)]
pub struct HashEmbedder;

impl HashEmbedder {
    pub fn new() -> Self {
        Self
    }

    /// Token byte spans: runs of non-whitespace, in order.
    fn tokens(text: &str) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i > start {
                spans.push((start, i));
            }
        }
        spans
    }

    /// A deterministic unit vector from a string.
    ///
    /// SHA-256 seeds a splitmix64 stream, which fills the vector, which
    /// is then L2 normalized. Every step is fixed-order integer work
    /// until the final normalization, so the result does not depend on
    /// the platform.
    fn vector(text: &str) -> Vec<f32> {
        let digest = Sha256::digest(text.as_bytes());
        let mut state = u64::from_be_bytes(digest[0..8].try_into().expect("32 byte digest"));
        let mut v = Vec::with_capacity(REQUIRED_DIMENSION as usize);
        for _ in 0..REQUIRED_DIMENSION {
            // splitmix64
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            // Map to [-1, 1) from the top 24 bits, which keeps the value
            // exactly representable in f32.
            let bits = (z >> 40) as u32;
            v.push((bits as f32 / 8_388_608.0) - 1.0);
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 && norm.is_finite() {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

impl Embedder for HashEmbedder {
    fn identity(&self) -> EmbedderIdentity {
        EmbedderIdentity {
            model: HASH_EMBEDDER_MODEL.to_string(),
            model_revision: "1".to_string(),
            dimension: REQUIRED_DIMENSION,
        }
    }

    fn max_tokens(&self) -> u32 {
        // High enough that no test document is refused for length, and
        // finite so the whole-text policy has a real number to use.
        1_000_000
    }

    async fn embed_document(
        &self,
        text: &str,
        _task: &str,
        chunking: ChunkPolicy,
    ) -> Result<Vec<EmbeddedChunk>, EmbeddingError> {
        let tokens = Self::tokens(text);
        if tokens.is_empty() {
            return Err(EmbeddingError::Service {
                status: 400,
                code: "invalid-request".into(),
                message: "no tokens in the input".into(),
            });
        }
        let n = tokens.len();
        let size = (chunking.size_tokens as usize).max(1);
        let step = size.saturating_sub(chunking.overlap_tokens as usize).max(1);

        // The windowing rule from the contract, which is
        // late_chunk_embeddings's rule.
        let mut windows = Vec::new();
        let mut start = 0usize;
        while start < n {
            let end = (start + size).min(n);
            windows.push((start, end));
            if end >= n {
                break;
            }
            start += step;
        }

        let total = windows.len() as u32;
        Ok(windows
            .into_iter()
            .enumerate()
            .map(|(i, (s, e))| {
                let start_byte = tokens[s].0;
                let end_byte = tokens[e - 1].1;
                EmbeddedChunk {
                    chunk_index: i as u32,
                    total_chunks: total,
                    vector: Self::vector(&text[start_byte..end_byte]),
                    start_token: s as u32,
                    end_token: e as u32,
                    start_byte,
                    end_byte,
                }
            })
            .collect())
    }
}

/// Turn a document's late chunks into `TextChunk`s and their vectors.
///
/// The one place the span contract is enforced. A chunk carries a byte span
/// into the caller's own text, and slicing by it is what proves the
/// service's character-to-byte conversion and its prefix rebase were right,
/// which is the seam the spike found a defect in. Both ingest paths and the
/// orchestrator called this shape, and three copies of a subtle check is
/// how the three drift.
///
/// The error is a `String` so each caller maps it into its own type: the
/// document path counts it as one file's refusal and carries on, the
/// codebase path fails the run.
pub fn late_pieces(
    text: &str,
    late: Vec<EmbeddedChunk>,
    label: &str,
) -> Result<(Vec<TextChunk>, Vec<Vec<f32>>), String> {
    let total = late.len();
    let mut chunks = Vec::with_capacity(total);
    let mut vectors = Vec::with_capacity(total);
    for (i, c) in late.into_iter().enumerate() {
        let slice = c.slice(text).ok_or_else(|| {
            format!(
                "{label}: chunk {i} spans bytes {}..{} which do not slice the text. \
                 The embedder's offset conversion is wrong",
                c.start_byte, c.end_byte
            )
        })?;
        chunks.push(TextChunk {
            text: slice.to_string(),
            start_char: c.start_byte,
            end_char: c.end_byte,
            chunk_index: i,
            total_chunks: total,
        });
        vectors.push(c.vector);
    }
    Ok((chunks, vectors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_double_is_deterministic_across_calls() {
        let e = HashEmbedder::new();
        let text = "the recursive descent parser reads tokens left to right";
        let a = e
            .embed_document(text, "retrieval.passage", ChunkPolicy::default())
            .await
            .unwrap();
        let b = e
            .embed_document(text, "retrieval.passage", ChunkPolicy::default())
            .await
            .unwrap();
        assert_eq!(a, b, "the same text embeds the same way every time");
    }

    #[tokio::test]
    async fn different_text_embeds_differently() {
        let e = HashEmbedder::new();
        let a = embed_one(&e, "postgres", "retrieval.query").await.unwrap();
        let b = embed_one(&e, "arangodb", "retrieval.query").await.unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), REQUIRED_DIMENSION as usize);
    }

    #[tokio::test]
    async fn every_vector_is_unit_norm() {
        let e = HashEmbedder::new();
        let text = (0..50)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = e
            .embed_document(
                &text,
                "retrieval.passage",
                ChunkPolicy {
                    size_tokens: 10,
                    overlap_tokens: 4,
                },
            )
            .await
            .unwrap();
        assert!(chunks.len() > 1, "50 tokens at 10/4 is several chunks");
        for c in &chunks {
            let norm: f32 = c.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-4,
                "chunk {} norm {norm}",
                c.chunk_index
            );
        }
    }

    /// The windowing rule, checked on the double so it is checked without
    /// a GPU. The oracle test checks the service against the same rule.
    #[tokio::test]
    async fn the_windows_tile_the_document_with_no_gap() {
        let e = HashEmbedder::new();
        let text = (0..37)
            .map(|i| format!("t{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = e
            .embed_document(
                &text,
                "retrieval.passage",
                ChunkPolicy {
                    size_tokens: 8,
                    overlap_tokens: 3,
                },
            )
            .await
            .unwrap();
        assert_eq!(chunks[0].start_token, 0);
        assert_eq!(chunks.last().unwrap().end_token, 37, "the last one clamps");
        for w in chunks.windows(2) {
            assert!(
                w[1].start_token < w[0].end_token,
                "consecutive windows overlap rather than leaving a gap"
            );
        }
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index, i as u32);
            assert_eq!(c.total_chunks, chunks.len() as u32);
        }
    }

    /// An overlap at or above the chunk size terminates, because the step
    /// has a floor of 1. Without that floor this loops forever, which is
    /// why the contract states the floor rather than leaving it implied.
    #[tokio::test]
    async fn an_overlap_at_or_above_the_size_terminates() {
        let e = HashEmbedder::new();
        let text = "a b c d e";
        for overlap in [5u32, 9] {
            let chunks = e
                .embed_document(
                    text,
                    "retrieval.passage",
                    ChunkPolicy {
                        size_tokens: 3,
                        overlap_tokens: overlap,
                    },
                )
                .await
                .unwrap();
            assert!(!chunks.is_empty());
            assert_eq!(chunks.last().unwrap().end_token, 5);
        }
    }

    /// Byte spans slice the caller's own text. Multi-byte input, because
    /// the spike found that ASCII hides a character-versus-byte mistake.
    #[tokio::test]
    async fn byte_spans_slice_multibyte_text() {
        let e = HashEmbedder::new();
        let text = "\u{4e16}\u{754c} \u{1f680}rocket \u{00e9}clair tail";
        let chunks = e
            .embed_document(
                text,
                "retrieval.passage",
                ChunkPolicy {
                    size_tokens: 2,
                    overlap_tokens: 0,
                },
            )
            .await
            .unwrap();
        for c in &chunks {
            let sliced = c.slice(text).expect("the span lands on a char boundary");
            assert!(!sliced.is_empty());
            assert!(text.contains(sliced));
        }
        assert_eq!(
            chunks[0].slice(text),
            Some("\u{4e16}\u{754c} \u{1f680}rocket")
        );
    }

    #[tokio::test]
    async fn an_empty_input_is_refused_rather_than_embedded_as_zero() {
        let e = HashEmbedder::new();
        for text in ["", "   \n\t "] {
            let err = e
                .embed_document(text, "retrieval.passage", ChunkPolicy::default())
                .await
                .expect_err("a zero vector has no cosine");
            assert_eq!(err.code(), Some("invalid-request"));
        }
    }

    #[tokio::test]
    async fn embed_one_pools_the_whole_text_as_one_chunk() {
        let e = HashEmbedder::new();
        let long = (0..3000)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let v = embed_one(&e, &long, "retrieval.query").await.unwrap();
        assert_eq!(v.len(), REQUIRED_DIMENSION as usize);
    }

    #[test]
    fn the_double_names_itself_a_double() {
        let id = HashEmbedder::new().identity();
        assert_eq!(id.model, "yeomna-test/hash-embedder");
        assert!(
            id.model.contains("test"),
            "a store holding these rows must say so: {}",
            id.model
        );
        assert_eq!(id.dimension, REQUIRED_DIMENSION);
    }

    #[test]
    fn tokens_are_whitespace_runs_with_real_spans() {
        let spans = HashEmbedder::tokens("  alpha\tbeta\n gamma  ");
        assert_eq!(spans.len(), 3);
        let text = "  alpha\tbeta\n gamma  ";
        assert_eq!(&text[spans[0].0..spans[0].1], "alpha");
        assert_eq!(&text[spans[2].0..spans[2].1], "gamma");
    }
}
