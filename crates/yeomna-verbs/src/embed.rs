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

/// Which query task pairs with which corpus task (spec 023).
///
/// A document and a search for it are asymmetric, which is why
/// `retrieval.passage` and `retrieval.query` exist as a pair. Embedding a
/// query the way the corpus was embedded produces a ranking that is
/// plausible and worse, with nothing in the result saying so, so the
/// pairing is read from the corpus rather than assumed.
///
/// `text-matching` is symmetric by construction: the model forces the query
/// prompt for it whatever it is asked, so both halves are the same name.
///
/// **`code` is provisional and R28 is the open question.** The model's
/// snapshot fixes no passage prompt for it, so the service uses the query
/// prompt for both halves and this table says the same. If `code` turns out
/// to be asymmetric, the contract grows `code.passage` and `code.query`,
/// this table gains one line, and every code corpus embedded under the old
/// answer is a re-ingest away from the new one. R26's `task` column is what
/// makes those corpora identifiable.
pub(crate) fn query_task_for(corpus_task: &str) -> Option<&'static str> {
    match corpus_task {
        "retrieval.passage" => Some("retrieval.query"),
        "retrieval.query" => Some("retrieval.query"),
        "text-matching" => Some("text-matching"),
        "code" => Some("code"),
        _ => None,
    }
}

/// Embed a search string as a query against a corpus embedded at
/// `corpus_task`, and return the vector as the literal the store's
/// `halfvec` cast takes.
///
/// The literal rather than the floats, because that is the only shape a
/// vector crosses into SQL in here (the sink does the same), and building
/// it in one place means the hybrid statement cannot get the format wrong
/// in a second one.
pub(crate) async fn query_vector_literal(
    s: &Exec<'_>,
    search_text: &str,
    corpus_task: &str,
) -> Result<String, VerbError> {
    let Some(task) = query_task_for(corpus_task) else {
        return Err(VerbError::Internal(format!(
            "the corpus was embedded at task {corpus_task:?}, which this appliance has no query \
             pairing for. Nothing can be ranked against it"
        )));
    };
    let client = s.embedder().await?;
    let info = client.info();
    if !info.loaded {
        return Err(VerbError::Internal(format!(
            "the embedder at {} is still loading its weights. Ask again in a moment",
            client.endpoint()
        )));
    }
    if !info.tasks.iter().any(|t| t == task) {
        return Err(VerbError::Internal(format!(
            "the corpus wants a {task:?} query and the embedder serves {}",
            info.tasks.join(", ")
        )));
    }
    let vector = client
        .embed_one(search_text, task)
        .await
        .map_err(|e| match e.code() {
            Some("input-too-large") | Some("invalid-request") => {
                VerbError::InvalidArgs(e.to_string())
            }
            _ => VerbError::Internal(e.to_string()),
        })?;
    let mut literal = String::with_capacity(vector.len() * 12 + 2);
    literal.push('[');
    for (i, v) in vector.iter().enumerate() {
        if i > 0 {
            literal.push(',');
        }
        literal.push_str(&v.to_string());
    }
    literal.push(']');
    Ok(literal)
}
