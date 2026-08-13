//! The response envelope, exactly as the CLI capture pinned it.
//!
//! `{success, command, data, timestamp}` on success and `{success: false,
//! command, error, timestamp}` on failure, timestamps RFC 3339 UTC. The
//! `data` shape is typed per verb by its implementing phase (spec 010
//! amendment): here it is the envelope's `serde_json::Value` slot.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::VerbError;

/// The wire envelope for every verb response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub success: bool,
    /// The verb's wire name.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// RFC 3339 UTC.
    pub timestamp: String,
}

/// Wrap a successful result.
pub fn envelope(command: &str, data: Value) -> Envelope {
    Envelope {
        success: true,
        command: command.to_string(),
        data: Some(data),
        error: None,
        timestamp: Utc::now().to_rfc3339(),
    }
}

/// Wrap a failure. The error string is the taxonomy's wire form, which is
/// also what the audit `outcome` column records.
pub fn error_envelope(command: &str, error: &VerbError) -> Envelope {
    Envelope {
        success: false,
        command: command.to_string(),
        data: None,
        error: Some(error.to_string()),
        timestamp: Utc::now().to_rfc3339(),
    }
}
