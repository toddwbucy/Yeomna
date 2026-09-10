//! The embedding client (spec 022, H4).
//!
//! Speaks `yeomna.embedding` v1, documented at
//! `docs/embedding-contract.md`, over HTTP/1.1 with JSON bodies on a Unix
//! domain socket. Three operations: `GET /v1/info`, `POST /v1/embed`, and
//! `POST /v1/tokens`.
//!
//! **The socket is the only transport this type can express.** Charter
//! section 5.1 says embedding is local and says it is a requirement
//! rather than a configuration default, so [`EmbeddingEndpoint`] holds a
//! filesystem path and has no variant that could name a host. The
//! previous shape defaulted to `http://localhost:8087/v1` and that
//! default, the constant behind it, and the test that pinned it all
//! retired together (PRD D10).
//!
//! **The response is always chunked.** There is no single-vector mode.
//! Late chunking encodes a document in one pass and decides afterwards
//! where the chunks were, so a response carries chunk vectors with both
//! their token ranges and their byte spans. [`EmbeddingClient::embed_one`]
//! is the whole-text case of the same operation rather than a second
//! shape.
//!
//! **The service pools, not this client.** PRD D1 rules the fork the
//! pipeline-libraries PRD deferred. `late_chunk_embeddings` in
//! `yeomna-chunking` is not the producer any more, it is the oracle the
//! service is checked against, which is what `POST /v1/tokens` exists
//! for.

use std::path::{Path, PathBuf};
use std::time::Duration;

use http::header::CONTENT_TYPE;
use http::{Method, Request, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

/// Request timeout. A whole-document forward pass on one GPU is slow and
/// the appliance would rather wait than retry.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// The dimension this appliance stores. `embeddings.vec` is
/// `halfvec(2048)`, which is not a preference: `vector(2048)` was
/// measured on this cluster to refuse an HNSW index. A service reporting
/// anything else is refused at connect rather than after an ingest.
pub const REQUIRED_DIMENSION: u32 = 2048;

/// Where the embedder answers.
///
/// A socket path and nothing else. There is deliberately no way to
/// construct this from a URL, which is how charter 5.1's local-embedding
/// requirement is held by the type system rather than by a default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingEndpoint(PathBuf);

impl EmbeddingEndpoint {
    /// Name a socket directly.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// The socket this endpoint names.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for EmbeddingEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

/// Configuration for the embedding client.
///
/// There is no `model` field. One service serves one model, `/v1/info`
/// says which, and a configured model name could disagree with the
/// loaded one in a way nothing would notice until two corpora turned out
/// to be incomparable.
#[derive(Debug, Clone)]
pub struct EmbeddingClientConfig {
    pub endpoint: EmbeddingEndpoint,
    pub timeout: Duration,
}

impl EmbeddingClientConfig {
    /// Point at a socket, with the default timeout.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            endpoint: EmbeddingEndpoint::new(path),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Read a socket out of a config value, refusing anything that is not
    /// one.
    ///
    /// This is the path a config file takes, so [`parse_endpoint`]'s
    /// refusal is reachable by an operator rather than only by a test. It
    /// also means `unix:///run/yeomna/embedder.sock`, the spelling the
    /// contract and this crate's own documentation use, resolves to the
    /// socket rather than to a literal relative path with `unix:` in it.
    pub fn from_config(value: &str) -> Result<Self, EmbeddingError> {
        Ok(Self {
            endpoint: parse_endpoint(value)?,
            timeout: DEFAULT_TIMEOUT,
        })
    }
}

/// Why an embedding call did not answer.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// The socket was not there, or the transport failed.
    #[error("embedder unreachable at {endpoint}: {reason}")]
    Unreachable { endpoint: String, reason: String },

    /// The service refused, in the contract's error envelope.
    #[error("embedder refused ({code}): {message}")]
    Service {
        status: u16,
        code: String,
        message: String,
    },

    /// The service answered something the contract does not describe.
    #[error("embedder response invalid: {0}")]
    InvalidResponse(String),

    /// The service did not answer in time.
    #[error("embedder timed out after {0}s")]
    Timeout(u64),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl EmbeddingError {
    /// Whether halving the batch and asking again is worth trying.
    ///
    /// Only out-of-memory qualifies. A refusal about the request itself
    /// is refused just as hard with fewer inputs, and retrying it would
    /// turn one clear error into several.
    pub fn is_retriable_oom(&self) -> bool {
        matches!(self, Self::Service { code, .. } if code == "out-of-memory")
    }

    /// The contract error code, when the service named one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Service { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl From<hyper_util::client::legacy::Error> for EmbeddingError {
    fn from(e: hyper_util::client::legacy::Error) -> Self {
        EmbeddingError::Unreachable {
            endpoint: "the configured socket".into(),
            reason: e.to_string(),
        }
    }
}

impl From<http::Error> for EmbeddingError {
    fn from(e: http::Error) -> Self {
        EmbeddingError::InvalidResponse(e.to_string())
    }
}

impl From<serde_json::Error> for EmbeddingError {
    fn from(e: serde_json::Error) -> Self {
        EmbeddingError::InvalidResponse(e.to_string())
    }
}

/// What `GET /v1/info` reports.
///
/// `model_revision` is the full model snapshot SHA. It is here rather
/// than assumed because it is what decides whether two corpora are
/// comparable, and a service that reloaded a different revision would
/// otherwise look identical.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub model: String,
    pub model_revision: String,
    pub dimension: u32,
    /// The service's real ceiling, not the model's advertised context.
    /// See the contract's ceiling section: 32k does not fit on this card.
    pub max_tokens: u32,
    pub tasks: Vec<String>,
    pub device: String,
    /// False while the weights are still loading, in which case `embed`
    /// refuses with `model-not-loaded` rather than blocking.
    pub loaded: bool,
}

/// One chunk of one input, with both coordinate systems.
///
/// The token range is what the pooling used. The byte span is what the
/// store holds, since `chunks.start_char` and `chunks.end_char` are byte
/// offsets. Both ride every chunk because the mapping between them is
/// where the spike found a defect, so a consumer can check one against
/// the other rather than trusting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedChunk {
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub vector: Vec<f32>,
    pub start_token: u32,
    pub end_token: u32,
    pub start_byte: usize,
    pub end_byte: usize,
}

impl EmbeddedChunk {
    /// The chunk's own text, sliced out of the input this chunk came
    /// from.
    ///
    /// Returns `None` when the span does not land on character
    /// boundaries, which means the service's offset conversion is wrong
    /// and is worth surfacing rather than panicking on.
    pub fn slice<'t>(&self, input: &'t str) -> Option<&'t str> {
        input.get(self.start_byte..self.end_byte)
    }
}

/// One input's chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedInput {
    /// Position in the request's input array. Redundant with position in
    /// `results` on purpose, so a mis-ordered response is detectable.
    pub index: usize,
    /// Tokens the forward pass saw, prefix excluded.
    pub token_count: u32,
    pub chunks: Vec<EmbeddedChunk>,
}

/// What `POST /v1/embed` returns.
#[derive(Debug, Clone)]
pub struct EmbedResult {
    pub model: String,
    pub model_revision: String,
    pub task: String,
    pub dimension: u32,
    /// One entry per input, in input order.
    pub results: Vec<EmbeddedInput>,
    pub duration_ms: u64,
}

/// What `POST /v1/tokens` returns. Its only consumer is the oracle test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStates {
    pub model: String,
    pub model_revision: String,
    pub task: String,
    pub dimension: u32,
    pub token_count: u32,
    pub prefix_bytes: usize,
    /// Byte pairs into the caller's own text, prefix already rebased.
    pub offsets: Vec<(usize, usize)>,
    /// `token_count` vectors of `dimension` floats.
    pub hidden_states: Vec<Vec<f32>>,
}

/// The client.
#[derive(Clone)]
pub struct EmbeddingClient {
    config: EmbeddingClientConfig,
    http: Client<UnixConnector, Full<Bytes>>,
    info: ServiceInfo,
}

impl EmbeddingClient {
    /// Connect, and learn what the service is.
    ///
    /// This calls `/v1/info`, so a client cannot exist without the
    /// service answering. That is deliberate: the alternative defers the
    /// discovery that the model is wrong until vectors are already in
    /// the store, and there is no re-embed-in-place tool to fix it with
    /// (T4).
    ///
    /// The dimension is checked here against [`REQUIRED_DIMENSION`],
    /// because `halfvec(2048)` is the column and a mismatch is not
    /// recoverable at write time in any useful way.
    #[instrument(skip_all, fields(socket = %config.endpoint))]
    pub async fn connect(config: EmbeddingClientConfig) -> Result<Self, EmbeddingError> {
        let socket = config.endpoint.path();
        // Not `Path::exists`: that answers false on EACCES too, and this
        // repo has already shipped one bug that way (CodeRabbit #43).
        match std::fs::metadata(socket) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(EmbeddingError::Unreachable {
                    endpoint: config.endpoint.to_string(),
                    reason: "no socket there. Is yeomna-embedder.service running".into(),
                });
            }
            Err(e) => {
                return Err(EmbeddingError::Unreachable {
                    endpoint: config.endpoint.to_string(),
                    reason: e.to_string(),
                });
            }
        }

        let http = Client::unix();
        let partial = Self {
            config,
            http,
            info: ServiceInfo {
                model: String::new(),
                model_revision: String::new(),
                dimension: 0,
                max_tokens: 0,
                tasks: Vec::new(),
                device: String::new(),
                loaded: false,
            },
        };
        let info = partial.fetch_info().await?;

        if info.dimension != REQUIRED_DIMENSION {
            return Err(EmbeddingError::InvalidResponse(format!(
                "the service serves {} dimensions and the store's column is halfvec({}). \
                 Vectors from this model cannot be written here",
                info.dimension, REQUIRED_DIMENSION
            )));
        }

        info!(
            model = %info.model,
            revision = %info.model_revision,
            device = %info.device,
            max_tokens = info.max_tokens,
            loaded = info.loaded,
            "embedder ready"
        );
        Ok(Self { info, ..partial })
    }

    /// Connect to a socket path.
    pub async fn connect_at(path: impl Into<PathBuf>) -> Result<Self, EmbeddingError> {
        Self::connect(EmbeddingClientConfig::at(path)).await
    }

    /// Connect to whatever a config file named, refusing a URL by name.
    pub async fn connect_configured(value: &str) -> Result<Self, EmbeddingError> {
        Self::connect(EmbeddingClientConfig::from_config(value)?).await
    }

    /// What the service said about itself at connect.
    pub fn info(&self) -> &ServiceInfo {
        &self.info
    }

    /// The socket this client speaks to.
    pub fn endpoint(&self) -> &EmbeddingEndpoint {
        &self.config.endpoint
    }

    /// Embed texts, late-chunked.
    ///
    /// `batch_size` splits the inputs across requests, which is how a
    /// long document list stays inside the card's headroom. On an
    /// out-of-memory refusal a multi-input batch is halved and retried,
    /// and results are reassembled by input index so the order does not
    /// depend on which sub-batch finished first.
    #[instrument(skip(self, texts), fields(count = texts.len(), task = %task))]
    pub async fn embed(
        &self,
        texts: &[String],
        task: &str,
        chunking: ChunkPolicy,
        batch_size: Option<u32>,
    ) -> Result<EmbedResult, EmbeddingError> {
        if texts.is_empty() {
            return Ok(EmbedResult {
                model: self.info.model.clone(),
                model_revision: self.info.model_revision.clone(),
                task: task.to_string(),
                dimension: self.info.dimension,
                results: Vec::new(),
                duration_ms: 0,
            });
        }

        let per_request = batch_size
            .filter(|&n| n > 0)
            .map(|n| n as usize)
            .unwrap_or(texts.len());

        use std::collections::{BTreeMap, VecDeque};
        let mut work: VecDeque<(usize, &[String])> = VecDeque::new();
        for (i, batch) in texts.chunks(per_request).enumerate() {
            work.push_back((i * per_request, batch));
        }

        let mut done: BTreeMap<usize, EmbeddedInput> = BTreeMap::new();
        let model = self.info.model.clone();
        let revision = self.info.model_revision.clone();
        let mut total_ms = 0u64;

        while let Some((start, batch)) = work.pop_front() {
            match self.embed_batch(batch, task, chunking).await {
                Ok(part) => {
                    // A batch split across requests must come back from one
                    // cohort. Taking whichever sub-batch answered last would
                    // silently mix two geometries into one result, and the
                    // store would record the wrong provenance for half of
                    // them with no way to tell afterwards.
                    if part.model != model || part.model_revision != revision {
                        return Err(EmbeddingError::InvalidResponse(format!(
                            "the service answered as {} @ {} and then as {} @ {} within one \
                             batch. Vectors from two cohorts cannot be compared",
                            model, revision, part.model, part.model_revision
                        )));
                    }
                    total_ms = total_ms.saturating_add(part.duration_ms);
                    for mut r in part.results {
                        // The service indexes within the sub-batch it
                        // was given, so rebase onto the caller's array.
                        r.index += start;
                        done.insert(r.index, r);
                    }
                }
                Err(e) if e.is_retriable_oom() && batch.len() > 1 => {
                    let mid = batch.len() / 2;
                    debug!(
                        start,
                        inputs = batch.len(),
                        "embedder out of memory, halving to {} and {}",
                        mid,
                        batch.len() - mid
                    );
                    work.push_front((start + mid, &batch[mid..]));
                    work.push_front((start, &batch[..mid]));
                }
                Err(e) => return Err(e),
            }
        }

        let results: Vec<EmbeddedInput> = done.into_values().collect();
        if results.len() != texts.len() {
            return Err(EmbeddingError::InvalidResponse(format!(
                "asked for {} inputs and got {} back",
                texts.len(),
                results.len()
            )));
        }
        for (i, r) in results.iter().enumerate() {
            if r.index != i {
                return Err(EmbeddingError::InvalidResponse(format!(
                    "result {i} claims index {}",
                    r.index
                )));
            }
        }

        Ok(EmbedResult {
            model,
            model_revision: revision,
            task: task.to_string(),
            dimension: self.info.dimension,
            results,
            duration_ms: total_ms,
        })
    }

    /// One document's chunks, which is the shape both ingest paths want.
    ///
    /// The convenience lives here rather than only on the pipeline's
    /// `Embedder` trait so this crate's own tests can reach it without
    /// depending on the pipeline, which depends on this crate.
    pub async fn embed_document(
        &self,
        text: &str,
        task: &str,
        chunking: ChunkPolicy,
    ) -> Result<Vec<EmbeddedChunk>, EmbeddingError> {
        let inputs = [text.to_string()];
        let result = self.embed(&inputs, task, chunking, None).await?;
        result
            .results
            .into_iter()
            .next()
            .map(|r| r.chunks)
            .ok_or_else(|| EmbeddingError::InvalidResponse("no result for the one input".into()))
    }

    /// One vector for one text, pooled over the whole thing.
    ///
    /// This is the single-window case of [`Self::embed`] rather than a
    /// second shape: the chunk size is the service's ceiling, so the
    /// windowing rule emits exactly one window clamped to the token
    /// count. It is what a query wants, and what `embed.text` returns.
    pub async fn embed_one(&self, text: &str, task: &str) -> Result<Vec<f32>, EmbeddingError> {
        let inputs = [text.to_string()];
        let result = self
            .embed(
                &inputs,
                task,
                ChunkPolicy::whole_text(self.info.max_tokens),
                None,
            )
            .await?;
        let chunks = result
            .results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::InvalidResponse("no result for the one input".into()))?
            .chunks;
        let mut chunks = chunks.into_iter();
        let first = chunks
            .next()
            .ok_or_else(|| EmbeddingError::InvalidResponse("no chunk for the one input".into()))?;
        if chunks.next().is_some() {
            return Err(EmbeddingError::InvalidResponse(
                "one window over the whole text produced more than one chunk".into(),
            ));
        }
        Ok(first.vector)
    }

    /// Token-level hidden states, for the oracle test and nothing else.
    ///
    /// Capped by the service at a small token count so it cannot become
    /// the production path by convenience. See the contract.
    pub async fn tokens(&self, text: &str, task: &str) -> Result<TokenStates, EmbeddingError> {
        let body = serde_json::json!({ "input": text, "task": task });
        let resp = self.request(Method::POST, "/tokens", Some(&body)).await?;
        serde_json::from_value(resp)
            .map_err(|e| EmbeddingError::InvalidResponse(format!("/v1/tokens: {e}")))
    }

    // ---------------------------------------------------------------------

    async fn fetch_info(&self) -> Result<ServiceInfo, EmbeddingError> {
        let resp = self.request(Method::GET, "/info", None).await?;
        serde_json::from_value(resp)
            .map_err(|e| EmbeddingError::InvalidResponse(format!("/v1/info: {e}")))
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        task: &str,
        chunking: ChunkPolicy,
    ) -> Result<EmbedResult, EmbeddingError> {
        let body = serde_json::json!({
            "input": texts,
            "task": task,
            "chunk_size_tokens": chunking.size_tokens,
            "chunk_overlap_tokens": chunking.overlap_tokens,
        });

        let started = std::time::Instant::now();
        let resp = self.request(Method::POST, "/embed", Some(&body)).await?;
        let duration_ms = started.elapsed().as_millis() as u64;

        let dimension = resp["dimension"].as_u64().unwrap_or(0) as u32;
        if dimension != self.info.dimension {
            return Err(EmbeddingError::InvalidResponse(format!(
                "the service reported {} dimensions at connect and {dimension} now",
                self.info.dimension
            )));
        }
        let results: Vec<EmbeddedInput> = serde_json::from_value(resp["results"].clone())
            .map_err(|e| EmbeddingError::InvalidResponse(format!("/v1/embed results: {e}")))?;

        // Every vector is the declared width, checked here rather than at
        // the store where the failure would be a Postgres cast error.
        for r in &results {
            for c in &r.chunks {
                if c.vector.len() as u32 != dimension {
                    return Err(EmbeddingError::InvalidResponse(format!(
                        "input {} chunk {} has {} floats, expected {dimension}",
                        r.index,
                        c.chunk_index,
                        c.vector.len()
                    )));
                }
            }
        }

        Ok(EmbedResult {
            model: resp["model"]
                .as_str()
                .unwrap_or(&self.info.model)
                .to_string(),
            model_revision: resp["model_revision"]
                .as_str()
                .unwrap_or(&self.info.model_revision)
                .to_string(),
            task: resp["task"].as_str().unwrap_or(task).to_string(),
            dimension,
            results,
            duration_ms,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, EmbeddingError> {
        let uri: Uri =
            hyperlocal::Uri::new(self.config.endpoint.path(), &format!("/v1{path}")).into();

        let body_bytes = match body {
            Some(v) => serde_json::to_vec(v)?,
            None => Vec::new(),
        };
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header(CONTENT_TYPE, "application/json");
        }
        let req = builder.body(Full::new(Bytes::copy_from_slice(&body_bytes)))?;

        let timeout = self.config.timeout;
        // One deadline covers headers and body, so a service that
        // answers promptly and then stalls the body still trips it.
        let (status, bytes) = tokio::time::timeout(timeout, async {
            let response = self.http.request(req).await?;
            let status = response.status();
            let bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| EmbeddingError::Unreachable {
                    endpoint: self.config.endpoint.to_string(),
                    reason: e.to_string(),
                })?
                .to_bytes();
            Ok::<_, EmbeddingError>((status, bytes))
        })
        .await
        .map_err(|_| EmbeddingError::Timeout(timeout.as_secs()))??;

        if !status.is_success() {
            return Err(self.service_error(status.as_u16(), &bytes));
        }

        serde_json::from_slice(&bytes)
            .map_err(|e| EmbeddingError::InvalidResponse(format!("unparseable response: {e}")))
    }

    /// Read the contract's error envelope, falling back to the raw body
    /// when the service answered something else.
    fn service_error(&self, status: u16, bytes: &[u8]) -> EmbeddingError {
        let parsed: Option<serde_json::Value> = serde_json::from_slice(bytes).ok();
        let (code, message) = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .map(|e| {
                (
                    e.get("code")
                        .and_then(|c| c.as_str())
                        .unwrap_or("internal")
                        .to_string(),
                    e.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            })
            .unwrap_or_else(|| {
                (
                    "internal".to_string(),
                    String::from_utf8_lossy(bytes).into_owned(),
                )
            });
        EmbeddingError::Service {
            status,
            code,
            message,
        }
    }
}

/// Where the chunk boundaries fall, in tokens.
///
/// These are request fields rather than service settings, so the service
/// executes a chunking policy and never owns one. The windowing rule is
/// `late_chunk_embeddings`'s rule, stated in the contract so two
/// implementations cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkPolicy {
    pub size_tokens: u32,
    pub overlap_tokens: u32,
}

impl ChunkPolicy {
    /// One window over the whole input, whatever its length, by asking
    /// for a window at least as wide as the service will accept.
    pub fn whole_text(max_tokens: u32) -> Self {
        Self {
            size_tokens: max_tokens.max(1),
            overlap_tokens: 0,
        }
    }
}

impl Default for ChunkPolicy {
    /// The reference's defaults, which the spike measured 29 chunks and
    /// a clean tiling with over 8,631 tokens.
    fn default() -> Self {
        Self {
            size_tokens: 500,
            overlap_tokens: 200,
        }
    }
}

impl std::fmt::Debug for EmbeddingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingClient")
            .field("endpoint", &self.config.endpoint)
            .field("model", &self.info.model)
            .field("revision", &self.info.model_revision)
            .finish()
    }
}

/// Read an endpoint out of a config string.
///
/// Accepts `unix:///path` and a bare absolute path. Rejects everything
/// else, including `http://localhost`, because charter 5.1 makes local
/// embedding a requirement rather than a default and a client that could
/// name a host would make it a default again.
pub fn parse_endpoint(s: &str) -> Result<EmbeddingEndpoint, EmbeddingError> {
    if let Some(path) = s.strip_prefix("unix://") {
        if path.starts_with('/') {
            return Ok(EmbeddingEndpoint::new(path));
        }
        return Err(EmbeddingError::Unreachable {
            endpoint: s.to_string(),
            reason: "unix:// needs an absolute path".into(),
        });
    }
    if s.starts_with('/') {
        return Ok(EmbeddingEndpoint::new(s));
    }
    let reason = if s.starts_with("http://") || s.starts_with("https://") {
        "the embedder is reached over a Unix socket and cannot be a URL. Charter 5.1 makes \
         local embedding a requirement rather than a configuration default, so this client \
         has no network transport to point at one"
            .to_string()
    } else {
        format!("expected unix:///path or an absolute path, got {s:?}")
    };
    Err(EmbeddingError::Unreachable {
        endpoint: s.to_string(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_not_an_endpoint_and_the_error_says_why() {
        for url in [
            "http://localhost:8087/v1",
            "https://api.example.com/v1",
            "http://10.0.0.5:8087",
        ] {
            let e = parse_endpoint(url).expect_err("must refuse");
            let said = e.to_string();
            assert!(said.contains("Unix socket"), "{said}");
            assert!(said.contains("5.1"), "it names the charter: {said}");
        }
    }

    #[test]
    fn a_socket_path_is_an_endpoint_either_way_it_is_written() {
        assert_eq!(
            parse_endpoint("unix:///run/yeomna/embedder.sock")
                .unwrap()
                .path(),
            Path::new("/run/yeomna/embedder.sock")
        );
        assert_eq!(
            parse_endpoint("/run/yeomna/embedder.sock").unwrap().path(),
            Path::new("/run/yeomna/embedder.sock")
        );
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert!(parse_endpoint("run/embedder.sock").is_err());
        assert!(parse_endpoint("unix://relative").is_err());
        assert!(parse_endpoint("").is_err());
    }

    /// The whole-text policy has to produce one window for any input the
    /// service will accept, which is what makes `embed_one` the
    /// single-window case rather than a second response shape.
    #[test]
    fn whole_text_policy_is_one_window() {
        let p = ChunkPolicy::whole_text(16384);
        assert_eq!(p.size_tokens, 16384);
        assert_eq!(p.overlap_tokens, 0);
        // The windowing rule: step is size minus overlap floored at 1,
        // and the first window's end clamps to the token count, so any
        // n <= size tiles in one window.
        let size = p.size_tokens as usize;
        let step = size.saturating_sub(p.overlap_tokens as usize).max(1);
        for n in [1usize, 7, 500, 16384] {
            assert_eq!(size.min(n), n, "one window covers {n}");
            assert!(step >= n, "and no second window starts inside {n}");
        }
    }

    #[test]
    fn a_zero_max_tokens_still_yields_a_usable_policy() {
        // A service that reported nonsense should not produce a policy
        // whose step is zero, which would loop forever upstream.
        assert_eq!(ChunkPolicy::whole_text(0).size_tokens, 1);
    }

    #[test]
    fn only_out_of_memory_is_worth_retrying() {
        let oom = EmbeddingError::Service {
            status: 503,
            code: "out-of-memory".into(),
            message: String::new(),
        };
        assert!(oom.is_retriable_oom());
        for code in [
            "input-too-large",
            "unknown-task",
            "model-not-loaded",
            "invalid-request",
        ] {
            let e = EmbeddingError::Service {
                status: 400,
                code: code.into(),
                message: String::new(),
            };
            assert!(!e.is_retriable_oom(), "{code} is not an OOM");
            assert_eq!(e.code(), Some(code));
        }
    }

    #[test]
    fn a_chunk_slices_the_text_it_came_from() {
        let c = EmbeddedChunk {
            chunk_index: 0,
            total_chunks: 1,
            vector: vec![],
            start_token: 0,
            end_token: 2,
            start_byte: 0,
            end_byte: 6,
        };
        assert_eq!(c.slice("hello world"), Some("hello "));
        // A span landing inside a multi-byte character is a service
        // defect, and it is reported rather than panicked on.
        let bad = EmbeddedChunk {
            end_byte: 1,
            ..c.clone()
        };
        assert_eq!(bad.slice("\u{4e16}\u{754c}"), None);
    }
}
