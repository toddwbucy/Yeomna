//! JSON-RPC plumbing and the per-request metadata the stateless core
//! requires (spec 024 FR13, FR14).
//!
//! The revision carries no `initialize` handshake, so nothing may be
//! inferred from an earlier request on the same connection. Every request
//! declares its protocol version and client capabilities in `_meta`, and
//! a request missing either is malformed rather than merely unusual.

use serde_json::{Value, json};

/// The revision this server implements natively, and the one it answers
/// `server/discover` with.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// Revisions this server also speaks, in the era that opens with an
/// `initialize` handshake instead of per-request metadata.
///
/// **This exists because a client refused to connect without it.** The
/// 2026-07-28 revision made the core stateless and dropped the handshake,
/// and a server that implements only that revision is unreachable to a
/// client that opens the older way. The specification's own compatibility
/// matrix names the cell: a legacy client against a modern server fails,
/// and "legacy clients have no fall-forward mechanism". A dual-era server
/// is explicitly permitted, and it is the only thing that makes this
/// surface usable today.
///
/// Newest first, because an unrecognized request gets the newest of these
/// as the server's counter-proposal, which is what version negotiation
/// asks for. The tool surface is two operations and does not vary across
/// any of them, which is why supporting them costs a handshake rather
/// than a second implementation.
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Is this request speaking the modern era, judged by the one key that
/// says so rather than by the extension block that carries it.
pub fn declares_modern(message: &Value) -> bool {
    message
        .get("params")
        .and_then(|p| p.get(META))
        .and_then(|m| m.get(VERSION_KEY))
        .is_some()
}

/// Which era a request is being served under.
#[derive(Debug, Clone, PartialEq)]
pub enum Era {
    /// Nothing has established an era yet. A request arriving here
    /// without metadata is the modern era's malformed case.
    Undetermined,
    /// An `initialize` handshake selected the older semantics, scoped to
    /// this process for the rest of its life.
    Legacy,
    /// The request carried per-request metadata.
    Modern,
}

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
/// The key whose presence means a request is speaking the modern era.
///
/// **`_meta` itself does not mean that**, and reading it that way was a
/// defect. `_meta` is the specification's open extension slot: a
/// `progressToken` lives there in every era, so a server that treats the
/// block's presence as a promise about its own keys refuses ordinary
/// client behaviour.
pub const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
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
pub fn result_response(id: &Value, mut result: Value, era: &Era) -> Value {
    // `resultType` and the per-response server identity belong to the
    // modern revision. The older era has no notion of either, so they are
    // not sent into it: a client that validates what it receives should
    // not have to tolerate fields from a revision it did not negotiate.
    if *era == Era::Modern
        && let Some(m) = result.as_object_mut()
    {
        m.insert("resultType".into(), json!("complete"));
        // Not an expect: a result that already carried a non-object here
        // would panic in the request path, and a panic is a worse answer
        // than a missing courtesy field.
        if let Some(meta) = m.entry(META).or_insert_with(|| json!({})).as_object_mut() {
            meta.insert(SERVER_INFO_KEY.into(), server_info());
        }
    }
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The negotiated version for an `initialize` request, and whether it is
/// one this server speaks.
///
/// Negotiation per the older lifecycle: answer with the requested version
/// when it is supported, and otherwise answer with the newest this server
/// supports and let the client decide whether to continue.
pub fn negotiate(requested: Option<&str>) -> Result<&'static str, RpcError> {
    let Some(requested) = requested else {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "initialize requires params.protocolVersion",
        ));
    };
    if let Some(v) = LEGACY_VERSIONS.iter().find(|v| **v == requested) {
        return Ok(v);
    }
    // The modern revision is a legal answer here too: a dual-era client
    // that opened the old way can be told this server prefers the new one.
    if requested == PROTOCOL_VERSION {
        return Ok(PROTOCOL_VERSION);
    }
    Ok(LEGACY_VERSIONS[0])
}

/// The handshake answer for the older era.
pub fn initialize_result(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    })
}

/// What a caller is told about this surface, in both eras.
pub const INSTRUCTIONS: &str = "Yeomna's verb contract as tools. Every call is audited on the appliance under the      calling user. Graph-scoped verbs take `graph` in their arguments. Verbs that read the      session's scope, `query` among them, take their graph and database from the appliance's      own config, so `query` with `hybrid` needs a config there naming both.";

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
            format!(
                "params.{META} is required and carries the per-request protocol fields. \
                 This server speaks {PROTOCOL_VERSION} that way, and it also speaks the \
                 older era if you open with an initialize handshake"
            ),
        )
        .with_data(json!({
            "supported": [PROTOCOL_VERSION],
            "supportedLegacy": LEGACY_VERSIONS,
        }))
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
    fn a_modern_result_carries_result_type_and_server_info() {
        let r = result_response(&json!(1), json!({"tools": []}), &Era::Modern);
        assert_eq!(r["result"]["resultType"], json!("complete"));
        assert_eq!(r["result"]["_meta"][SERVER_INFO_KEY]["name"], "yeomna-mcp");
    }

    #[test]
    fn a_legacy_result_carries_neither() {
        let r = result_response(&json!(1), json!({"tools": []}), &Era::Legacy);
        assert!(r["result"].get("resultType").is_none());
        assert!(r["result"].get("_meta").is_none());
    }

    #[test]
    fn negotiation_echoes_a_supported_version_and_counter_offers_otherwise() {
        assert_eq!(negotiate(Some("2025-11-25")).unwrap(), "2025-11-25");
        assert_eq!(negotiate(Some("2025-06-18")).unwrap(), "2025-06-18");
        assert_eq!(negotiate(Some(PROTOCOL_VERSION)).unwrap(), PROTOCOL_VERSION);
        // Unknown gets this server's newest handshake version rather than
        // a refusal, which is what the older lifecycle asks for.
        assert_eq!(negotiate(Some("1.0.0")).unwrap(), LEGACY_VERSIONS[0]);
        assert!(negotiate(None).is_err(), "the version is required");
    }

    #[test]
    fn the_modern_metadata_error_names_both_eras() {
        // A client that cannot connect may only ever see this string.
        let e = check_meta(None).unwrap_err();
        let d = e.data.expect("the error carries what is supported");
        assert_eq!(d["supported"], json!([PROTOCOL_VERSION]));
        assert_eq!(d["supportedLegacy"][0], json!(LEGACY_VERSIONS[0]));
        assert!(e.message.contains("initialize handshake"), "{}", e.message);
    }

    #[test]
    fn cache_hints_are_a_non_negative_integer_and_a_valid_scope() {
        let r = with_cache_hints(json!({}));
        assert!(r["ttlMs"].as_u64().is_some(), "ttlMs is an integer >= 0");
        assert_eq!(r["cacheScope"], json!("public"));
    }
}
