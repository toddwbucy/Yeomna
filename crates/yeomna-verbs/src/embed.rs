//! `embed.text`, the verb that used to name H4 (spec 022).
//!
//! One text in, one vector out, pooled over the whole thing. That is the
//! single-window case of the same late-chunking operation the ingest paths
//! use rather than a second shape, so there is no separate code path here
//! that could pool differently.
//!
//! The default task is `retrieval.query`. A bare `embed.text` is
//! overwhelmingly a caller embedding something to search with, and a
//! query embedded at `retrieval.passage` is the silent failure the
//! reference's own contract warned about: the vectors are plausible, the
//! ranking is quietly worse, and nothing errors. The corpus half is named
//! explicitly by whatever ingested it.
//!
//! The response carries the model, the revision, and the task, because a
//! vector whose cohort is unknown cannot be compared to anything (R26).

use serde_json::{Value, json};

use crate::error::VerbError;
use crate::execute::Exec;
use crate::verb::EmbedTextRequest;

/// The task a caller gets when they name none.
pub(crate) const DEFAULT_TASK: &str = "retrieval.query";

/// `embed.text`: a vector for a text.
pub(crate) async fn text(s: &Exec<'_>, r: &EmbedTextRequest) -> Result<Value, VerbError> {
    if r.text.trim().is_empty() {
        // A zero vector has no cosine and would answer every query
        // equally badly from inside an HNSW index, so an empty text is a
        // refusal rather than a degenerate answer.
        return Err(VerbError::InvalidArgs("text is empty".into()));
    }
    let task = r.task.as_deref().unwrap_or(DEFAULT_TASK);
    let client = s.embedder().await?;
    let info = client.info();
    if !info.tasks.iter().any(|t| t == task) {
        return Err(VerbError::InvalidArgs(format!(
            "the embedder does not serve task {task:?}. It serves {}",
            info.tasks.join(", ")
        )));
    }
    let model = info.model.clone();
    let model_revision = info.model_revision.clone();
    let dimension = info.dimension;

    let vector = client.embed_one(&r.text, task).await.map_err(|e| {
        // A refusal the caller can fix reads back as their own, and
        // everything else is the appliance's.
        match e.code() {
            Some("input-too-large") | Some("unknown-task") | Some("invalid-request") => {
                VerbError::InvalidArgs(e.to_string())
            }
            _ => VerbError::Internal(e.to_string()),
        }
    })?;

    Ok(json!({
        "model": model,
        "model_revision": model_revision,
        "task": task,
        "dimension": dimension,
        "vector": vector,
    }))
}
