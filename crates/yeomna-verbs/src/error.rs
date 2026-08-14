//! The error taxonomy, five kinds, stable wire strings.
//!
//! Two renderings, deliberately different. [`VerbError::kind`] is the
//! stable vocabulary word, and the audit `outcome` column records
//! `failed: <kind>` with no detail, because an audit column is a
//! vocabulary rather than a message log. [`std::fmt::Display`] is
//! `kind: detail` and rides the response envelope, where the caller
//! wants to know which thing was not found. The Phase 4 audit writer
//! uses `kind`, and `schema.sql` states the same contract.

use serde::{Deserialize, Serialize};

/// Everything a verb can fail with, as the contract sees it.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "kebab-case")]
pub enum VerbError {
    /// The addressed thing does not exist.
    #[error("not-found: {0}")]
    NotFound(String),
    /// The request was well-formed JSON and a known verb, and still wrong.
    #[error("invalid-args: {0}")]
    InvalidArgs(String),
    /// The verb's contract exists and its implementation does not yet
    /// (graph-embed before H9, ingest before H3).
    #[error("unimplemented: {0}")]
    Unimplemented(String),
    /// Refused by policy: the sql verb against a KG-pattern database, a
    /// role without the grant, a destructive verb without its
    /// acknowledgement.
    #[error("denied: {0}")]
    Denied(String),
    /// The appliance's fault, never the caller's.
    #[error("internal: {0}")]
    Internal(String),
}

impl VerbError {
    /// The stable kind string: the audit outcome's failure name.
    pub fn kind(&self) -> &'static str {
        match self {
            VerbError::NotFound(_) => "not-found",
            VerbError::InvalidArgs(_) => "invalid-args",
            VerbError::Unimplemented(_) => "unimplemented",
            VerbError::Denied(_) => "denied",
            VerbError::Internal(_) => "internal",
        }
    }
}
