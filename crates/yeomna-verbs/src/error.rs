//! The error taxonomy, five kinds, stable wire strings.
//!
//! The kind string doubles as the audit `outcome` failure name (V-Q1:
//! attempt logging, a failed verb marks its row `failed: <kind>`).

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
