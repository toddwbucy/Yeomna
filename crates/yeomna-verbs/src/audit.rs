//! The audit write, per V-Q1 and charter section 6.
//!
//! Every verb call leaves a row, reads included, because the charter says
//! human and agent are logged the same way and does not carve reads out.
//! The mechanism here is the one every later phase inherits.
//!
//! **The row commits before the verb runs.** That is what makes V-Q1
//! attempt logging rather than completed-action logging: a call that
//! crashes mid-flight leaves a row with a NULL outcome, which reads as
//! exactly what happened, where a row written afterwards would leave no
//! trace of the attempt at all.
//!
//! Phase 4 adds the transactional coupling of an audit row with a
//! mutation and its diff-log entry. This module is the row alone.

use serde_json::Value;
use tokio_postgres::Client;

use crate::error::VerbError;

/// A recorded attempt, waiting for its outcome.
#[derive(Debug, Clone, Copy)]
pub struct Attempt {
    id: i64,
}

impl Attempt {
    /// The row's id, for the write path that marks its own outcome
    /// inside the mutation transaction (spec 014 FR5).
    pub(crate) fn id(&self) -> i64 {
        self.id
    }
}

/// Record the attempt. The verb does not run if this fails (EC-5): a call
/// that cannot be recorded is a call the appliance declines to make,
/// which is what one audited entry point costs when the log is
/// unavailable.
pub async fn begin(
    client: &Client,
    actor: &str,
    verb: &str,
    args: &Value,
) -> Result<Attempt, VerbError> {
    let row = client
        .query_one(
            "INSERT INTO audit_log (actor, verb, args)
             VALUES ($1, $2, $3::text::jsonb) RETURNING id",
            &[&actor, &verb, &args.to_string()],
        )
        .await
        .map_err(|e| VerbError::Internal(format!("the audit log refused the attempt: {e}")))?;
    Ok(Attempt { id: row.get(0) })
}

/// Mark how the attempt ended.
///
/// The column records the taxonomy's kind and never the error detail: an
/// audit column is a stable vocabulary, and the detail rides the response
/// envelope where the caller wants it.
///
/// A failure to mark is logged rather than raised. The verb has already
/// run and its result is the caller's, and the row left behind with a
/// NULL outcome is the same shape a crash leaves, which is honest.
pub async fn finish(client: &Client, attempt: Attempt, outcome: Result<(), &VerbError>) {
    let mark = match outcome {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("failed: {}", e.kind()),
    };
    if let Err(e) = client
        .execute(
            // The NULL guard makes the first mark the only mark: a write
            // verb that committed 'ok' inside its transaction is not
            // overwritten by the generic pass that follows dispatch.
            "UPDATE audit_log SET outcome = $1 WHERE id = $2 AND outcome IS NULL",
            &[&mark, &attempt.id],
        )
        .await
    {
        tracing::warn!(%e, id = attempt.id, "could not mark the audit outcome");
    }
}
