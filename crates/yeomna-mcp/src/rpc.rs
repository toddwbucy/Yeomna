//! JSON-RPC plumbing and the per-request metadata the stateless core
//! requires (spec 024 FR13, FR14).
//!
//! The revision carries no `initialize` handshake, so nothing may be
//! inferred from an earlier request on the same connection. Every request
//! declares its protocol version and client capabilities in `_meta`, and
//! a request missing either is malformed rather than merely unusual.

use serde_json::{Value, json};

/// The revision this server implements.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// How long a client may consider the tool list fresh. The list is
/// derived from a compiled-in enum, so it cannot change while this
/// process runs, and an hour is a freshness hint rather than a promise.
pub const TTL_MS: u64 = 3_600_000;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// Defined by MCP, and usable only with its specified meaning.
pub const UNSUPPORTED_VERSION: i64 = -32022;

// MCP also defines `MissingRequiredClientCapability` (-32021), for a
// request whose handling needs a capability the client did not declare.
// **No operation on this surface needs one**: `server/discover`,
// `tools/list`, and `tools/call` are answered from the contract and a
// subprocess, and none of them asks anything of the client. So the code
// has no path that raises it, and a constant sitting here unused would
// imply one exists. It belongs with the first operation that needs a
// client capability, which on this surface would be a new feature rather
// than a new error.

const META: &str = "_meta";
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";

/// This server's self-reported identity. Display and logging only, per
/// the revision's note that it is unverified.
pub fn server_info() -> Value {
    json!({"name": "yeomna-mcp", "version": env!("CARGO_PKG_VERSION")})
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    fn to_json(&self) -> Value {
        let mut e = json!({"code": self.code, "message": self.message});
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        e
    }
}

/// A full error response, ready to write.
pub fn error_response(id: Option<&Value>, e: &RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": e.to_json(),
    })
}

/// A full result response. `resultType` is set here and nowhere else, so
/// no response path can forget it (FR13): the field belongs to the
/// envelope MCP wraps rather than to anything Yeomna produces, which is
/// exactly why it is easy to omit.
pub fn result_response(id: &Value, mut result: Value) -> Value {
    if let Some(m) = result.as_object_mut() {
        m.insert("resultType".into(), json!("complete"));
        let meta = m
            .entry(META)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("_meta is an object");
        meta.insert(SERVER_INFO_KEY.into(), server_info());
    }
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Caching hints, which the revision requires on cacheable results and
/// forbids nowhere else that matters here: `tools/call` is not a
/// cacheable operation, so a verb result never passes through this.
pub fn with_cache_hints(mut result: Value) -> Value {
    if let Some(m) = result.as_object_mut() {
        m.insert("ttlMs".into(), json!(TTL_MS));
        m.insert("cacheScope".into(), json!("public"));
    }
    result
}

/// The required per-request metadata, checked before a method runs.
///
/// `protocolVersion` and `clientCapabilities` are both required and
/// `clientInfo` is not, so a request carrying only the first two is
/// well formed. A missing or malformed required field is `-32602`, and an
/// unsupported version is `-32022` listing what this server does support.
pub fn check_meta(params: Option<&Value>) -> Result<(), RpcError> {
    let meta = params.and_then(|p| p.get(META)).ok_or_else(|| {
        RpcError::new(
            INVALID_PARAMS,
            format!("params.{META} is required and carries the per-request protocol fields"),
        )
    })?;
    let version = meta.get(VERSION_KEY).ok_or_else(|| {
        RpcError::new(INVALID_PARAMS, format!("{META}.{VERSION_KEY} is required"))
    })?;
    let version = version.as_str().ok_or_else(|| {
        RpcError::new(
            INVALID_PARAMS,
            format!("{META}.{VERSION_KEY} must be a string"),
        )
    })?;
    match meta.get(CAPABILITIES_KEY) {
        None => {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("{META}.{CAPABILITIES_KEY} is required"),
            ));
        }
        Some(c) if !c.is_object() => {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("{META}.{CAPABILITIES_KEY} must be an object"),
            ));
        }
        Some(_) => {}
    }
    // Capabilities are required and are checked for shape, not for
    // content, because nothing here needs a capability from the client.
    if version != PROTOCOL_VERSION {
        return Err(
            RpcError::new(UNSUPPORTED_VERSION, "Unsupported protocol version").with_data(json!({
                "supported": [PROTOCOL_VERSION],
                "requested": version,
            })),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(v: Value) -> Value {
        json!({ META: v })
    }

    fn good() -> Value {
        meta(json!({VERSION_KEY: PROTOCOL_VERSION, CAPABILITIES_KEY: {}}))
    }

    #[test]
    fn a_well_formed_request_passes_without_client_info() {
        assert!(check_meta(Some(&good())).is_ok());
    }

    #[test]
    fn absent_meta_is_invalid_params() {
        assert_eq!(check_meta(None).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(
            check_meta(Some(&json!({}))).unwrap_err().code,
            INVALID_PARAMS
        );
    }

    #[test]
    fn each_required_field_is_required_on_its_own() {
        let no_version = meta(json!({CAPABILITIES_KEY: {}}));
        assert_eq!(
            check_meta(Some(&no_version)).unwrap_err().code,
            INVALID_PARAMS
        );
        let no_caps = meta(json!({VERSION_KEY: PROTOCOL_VERSION}));
        assert_eq!(check_meta(Some(&no_caps)).unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn a_malformed_required_field_is_invalid_params_and_not_a_version_error() {
        // EC-10: protocolVersion as a number is malformed, which is
        // -32602, rather than an unsupported version, which is -32022.
        let numeric = meta(json!({VERSION_KEY: 20260728, CAPABILITIES_KEY: {}}));
        assert_eq!(check_meta(Some(&numeric)).unwrap_err().code, INVALID_PARAMS);
        let caps = meta(json!({VERSION_KEY: PROTOCOL_VERSION, CAPABILITIES_KEY: "yes"}));
        assert_eq!(check_meta(Some(&caps)).unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn an_unsupported_version_names_what_is_supported() {
        let old = meta(json!({VERSION_KEY: "1900-01-01", CAPABILITIES_KEY: {}}));
        let e = check_meta(Some(&old)).unwrap_err();
        assert_eq!(e.code, UNSUPPORTED_VERSION);
        let data = e.data.unwrap();
        assert_eq!(data["supported"], json!([PROTOCOL_VERSION]));
        assert_eq!(data["requested"], json!("1900-01-01"));
    }

    #[test]
    fn every_result_carries_result_type_and_server_info() {
        let r = result_response(&json!(1), json!({"tools": []}));
        assert_eq!(r["result"]["resultType"], json!("complete"));
        assert_eq!(r["result"]["_meta"][SERVER_INFO_KEY]["name"], "yeomna-mcp");
    }

    #[test]
    fn cache_hints_are_a_non_negative_integer_and_a_valid_scope() {
        let r = with_cache_hints(json!({}));
        assert!(r["ttlMs"].as_u64().is_some(), "ttlMs is an integer >= 0");
        assert_eq!(r["cacheScope"], json!("public"));
    }
}
