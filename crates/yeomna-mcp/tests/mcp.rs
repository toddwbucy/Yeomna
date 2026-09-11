//! The stdio server, driven over pipes the way a client drives it.
//!
//! Every test here stands a fake `yeomna` (and, where the target is ssh,
//! a fake `ssh`) in a temporary directory, so the protocol surface is
//! exercised without a cluster. What the appliance does with a request is
//! the verb layer's business and is tested there. What this crate owes is
//! that the request arrives unaltered, the answer comes back as the right
//! kind of MCP message, and nothing but MCP messages reaches stdout.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use yeomna_mcp::{Target, serve, tools};

const VERSION: &str = "2026-07-28";

/// A well-formed request, since every one of them needs the same `_meta`.
fn request(id: i64, method: &str, params: Value) -> String {
    let mut p = params;
    p["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": VERSION,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": p}).to_string()
}

/// Write an executable stand-in and return its path.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    write!(f, "#!/bin/sh\n{body}").unwrap();
    drop(f);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// An envelope the way `yeomna call` prints one.
const OK_ENVELOPE: &str = r#"{"success":true,"command":"status","data":{"actor":"todd"},"timestamp":"2026-09-11T00:00:00Z"}"#;

struct Client {
    to_server: DuplexStream,
    from_server: BufReader<DuplexStream>,
}

impl Client {
    fn start(target: Target) -> Self {
        let (to_server, server_reads) = tokio::io::duplex(64 * 1024);
        let (server_writes, from_server) = tokio::io::duplex(1024 * 1024);
        tokio::spawn(async move {
            let _ = serve(server_reads, server_writes, target).await;
        });
        Self {
            to_server,
            from_server: BufReader::new(from_server),
        }
    }

    async fn send(&mut self, line: &str) {
        self.to_server.write_all(line.as_bytes()).await.unwrap();
        self.to_server.write_all(b"\n").await.unwrap();
        self.to_server.flush().await.unwrap();
    }

    /// One message, or a failure that says so rather than hanging.
    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            self.from_server.read_line(&mut line),
        )
        .await
        .expect("the server answered within twenty seconds")
        .unwrap();
        assert!(read > 0, "the server closed without answering");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not a JSON-RPC line: {e}: {line:?}"))
    }

    /// Close the input, which is the client's shutdown signal (FR11).
    async fn close(&mut self) {
        self.to_server.shutdown().await.unwrap();
    }
}

fn local(dir: &Path, body: &str) -> Target {
    Target::local().with_program(script(dir, "yeomna", body).display().to_string())
}

// -- discovery and the tool list --------------------------------------

#[tokio::test]
async fn discover_reports_the_version_and_the_tools_capability() {
    // FR1, FR13, FR17.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(1, "server/discover", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["supportedVersions"], json!([VERSION]));
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    assert!(r["result"]["ttlMs"].as_u64().is_some());
    assert_eq!(r["result"]["cacheScope"], "public");
}

#[tokio::test]
async fn tools_list_is_the_contract_minus_the_excluded_verb() {
    // FR2, FR2a, FR17.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(1, "tools/list", json!({}))).await;
    let r = c.recv().await;
    let tools = r["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 41);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"sql"), "sql is on the surface (R29)");
    assert!(names.contains(&"query"), "the abstractions are all here");
    for t in tools {
        assert!(!t["description"].as_str().unwrap().is_empty());
        assert_eq!(t["inputSchema"]["type"], "object");
    }
    assert_eq!(r["result"]["cacheScope"], "public");
}

#[tokio::test]
async fn the_excluded_verb_is_unreachable_through_dispatch_too() {
    // FR2b, and the side door this guards: a server that filtered its
    // list and dispatched from the whole enum would ship a tool nobody
    // advertises and anybody can call.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    for excluded in tools::EXCLUDED {
        c.send(&request(
            1,
            "tools/call",
            json!({"name": excluded, "arguments": {"database": "x", "statement": "y"}}),
        ))
        .await;
        let r = c.recv().await;
        assert_eq!(r["error"]["code"], -32601, "{excluded} must be unknown");
        assert!(r.get("result").is_none(), "no result for {excluded}");
    }
}

// -- calling a verb ----------------------------------------------------

#[tokio::test]
async fn an_answer_arrives_as_structured_content() {
    // FR4, FR13.
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        7,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["id"], 7);
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["structuredContent"]["data"]["actor"], "todd");
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("todd")
    );
    // A verb result is not a cacheable operation, so it carries no hints.
    assert!(r["result"].get("ttlMs").is_none());
    assert!(r["result"].get("cacheScope").is_none());
}

#[tokio::test]
async fn a_refusal_is_a_result_the_model_can_read_and_not_a_protocol_error() {
    // FR5. Yeomna's refusals are written to be read.
    let d = tempfile::tempdir().unwrap();
    let envelope = r#"{"success":false,"command":"query","error":"hybrid ranking needs a graph","timestamp":"t"}"#;
    let body = format!("cat > /dev/null\nprintf '%s' '{envelope}'\nexit 1");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        2,
        "tools/call",
        json!({"name": "query", "arguments": {"search_text": "x", "hybrid": true}}),
    ))
    .await;
    let r = c.recv().await;
    assert!(
        r.get("error").is_none(),
        "a refusal is not a JSON-RPC error"
    );
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["resultType"], "complete");
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("hybrid ranking needs a graph"),
        "got {text:?}"
    );
}

#[tokio::test]
async fn an_unknown_tool_is_a_protocol_error() {
    // FR6.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(3, "tools/call", json!({"name": "not.a.verb"})))
        .await;
    let r = c.recv().await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn a_failure_with_nothing_on_stdout_is_never_an_empty_success() {
    // EC-1. A test that checked only the result would read a crash as an
    // empty answer.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(
        d.path(),
        "cat > /dev/null\necho 'it broke' >&2\nexit 1",
    ));
    c.send(&request(
        4,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    assert!(r.get("result").is_none(), "not a success");
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("said nothing on stdout"), "got {m:?}");
    assert!(m.contains("it broke"), "the error output is quoted: {m:?}");
}

// -- the transport -----------------------------------------------------

#[tokio::test]
async fn the_request_reaches_the_child_on_stdin_byte_for_byte() {
    // FR15, and the defect review caught: ssh joins its command
    // arguments into one string and hands it to a shell on the far side,
    // so a request carrying these characters in argv would be
    // interpreted there rather than delivered.
    let d = tempfile::tempdir().unwrap();
    let capture = d.path().join("seen.json");
    let body = format!(
        "cat > '{}'\nprintf '%s' '{OK_ENVELOPE}'\nexit 0",
        capture.display()
    );
    let mut c = Client::start(local(d.path(), &body));
    let nasty = "'; rm -rf / #\" $(id) `whoami` ${HOME}\nsecond line";
    c.send(&request(
        5,
        "tools/call",
        json!({"name": "query", "arguments": {"search_text": nasty}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false);

    let seen: Value = serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
    assert_eq!(seen["verb"], "query");
    assert_eq!(
        seen["args"]["search_text"], nasty,
        "the text arrived unaltered and no shell read it as syntax"
    );
}

#[tokio::test]
async fn the_child_sees_end_of_input_after_exactly_one_request() {
    // FR18. `yeomna call -` reads until end of input, so a child whose
    // stdin stays open waits rather than answering. The symptom of
    // omitting the close is a hang, which is why this stand-in blocks
    // until EOF and the assertion is that the call returns at all.
    let d = tempfile::tempdir().unwrap();
    let body = format!("payload=$(cat)\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        6,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false, "the call completed");
}

#[tokio::test]
async fn a_child_that_floods_stderr_does_not_deadlock_the_call() {
    // Found in self-review, not by the spec. Draining stdout to end of
    // input before touching stderr deadlocks a child that fills the
    // stderr pipe buffer: it blocks writing stderr, so it never exits, so
    // stdout never reaches end of input. An ingest that logs is exactly
    // that child, and the symptom is a hang rather than an error.
    let d = tempfile::tempdir().unwrap();
    // Comfortably past a 64 KiB pipe buffer.
    let body = format!(
        "cat > /dev/null\ni=0\nwhile [ $i -lt 4000 ]; do \
         echo 'a log line long enough to matter for the pipe buffer' >&2; i=$((i+1)); done\n\
         printf '%s' '{OK_ENVELOPE}'\nexit 0"
    );
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        41,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["id"], 41);
    assert_eq!(r["result"]["isError"], false, "the call completed");
}

#[tokio::test]
async fn ssh_failing_is_reported_as_the_transport_and_not_as_a_refusal() {
    // FR16, EC-3. `yeomna call` returns only 0, 1, or 2, so 255 is
    // distinguishable, and a caller who cannot tell transport from
    // appliance debugs the wrong machine.
    let d = tempfile::tempdir().unwrap();
    let fake_ssh = script(
        d.path(),
        "ssh",
        "cat > /dev/null\necho 'ssh: connect to host olympus: No route to host' >&2\nexit 255",
    );
    let target = Target::ssh("olympus").with_ssh_program(fake_ssh.display().to_string());
    let mut c = Client::start(target);
    c.send(&request(
        8,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("ssh could not reach the appliance"), "got {m:?}");
    assert!(m.contains("olympus"), "names the destination: {m:?}");
}

#[tokio::test]
async fn an_unexpected_exit_status_is_reported_as_itself() {
    // FR16's other half: a status this contract does not define is not
    // folded into a verb outcome.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "cat > /dev/null\nexit 42"));
    c.send(&request(
        9,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("exited 42"), "got {m:?}");
}

// -- the stdio binding -------------------------------------------------

#[tokio::test]
async fn nothing_but_mcp_messages_reaches_stdout() {
    // FR7, D11. The hazard is specific: `yeomna call` prints its
    // envelope to stdout and the CLI tree puts headers and rows there
    // together, so a child whose stdout were inherited would put a table
    // on the MCP channel.
    let d = tempfile::tempdir().unwrap();
    let body = format!(
        "cat > /dev/null\nprintf 'KIND\\tKEY\\nnode\\ta\\n'\nprintf '%s' '{OK_ENVELOPE}'\n\
         echo 'a log line' >&2\nexit 0"
    );
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        10,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    // The child's table went to its captured stdout, so the only thing
    // on this stream is one JSON-RPC line. The envelope did not parse,
    // which is EC-2 and an error rather than a leak.
    let r = c.recv().await;
    assert!(r["jsonrpc"] == "2.0", "every line is JSON-RPC: {r}");
    assert!(r.get("error").is_some(), "the mixed output did not parse");
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("not an envelope"), "got {m:?}");
    assert!(m.contains("KIND"), "it quotes what arrived: {m:?}");
}

#[tokio::test]
async fn end_of_input_finishes_work_already_running_before_it_exits() {
    // Found by dogfooding, not by the spec. The first build cancelled
    // in-flight calls on EOF, so a client that wrote its requests and
    // closed the stream lost every answer whose call was still running,
    // and a verb that had already committed left a NULL audit outcome
    // for work that succeeded. Abandoning committed work is the client's
    // call to make explicitly, never something to infer from a closed
    // pipe.
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nsleep 1\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        40,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    // Close immediately, while the call is still running.
    c.close().await;
    let r = c.recv().await;
    assert_eq!(r["id"], 40, "the answer survived the close");
    assert_eq!(r["result"]["isError"], false);
}

#[tokio::test]
async fn end_of_input_ends_the_session() {
    // FR11.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(11, "tools/list", json!({}))).await;
    let _ = c.recv().await;
    c.close().await;
    let mut line = String::new();
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        c.from_server.read_line(&mut line),
    )
    .await
    .expect("the server exited promptly on end of input")
    .unwrap();
    assert_eq!(n, 0, "the stream closed");
}

#[tokio::test]
async fn a_cancelled_call_is_killed_and_answered_with_nothing() {
    // FR10, D10. The revision says stop and send nothing further for
    // that id.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "cat > /dev/null\nsleep 120\nexit 0"));
    c.send(&request(
        12,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    // Give the child a moment to exist before cancelling it.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    c.send(
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": 12}
        })
        .to_string(),
    )
    .await;
    // Nothing for 12, and the session still answers other work, which is
    // how we know the cancellation did not take the server with it.
    c.send(&request(13, "tools/list", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["id"], 13, "the only answer is the later request");
}

#[tokio::test]
async fn concurrent_calls_each_get_their_own_child() {
    // EC-6. Each call spawns its own `yeomna call`, its own connection,
    // and its own session, so the appliance's per-session serialization
    // is unaffected.
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nsleep 1\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    let started = std::time::Instant::now();
    for id in 20..24 {
        c.send(&request(
            id,
            "tools/call",
            json!({"name": "status", "arguments": {}}),
        ))
        .await;
    }
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(c.recv().await["id"].as_i64().unwrap());
    }
    seen.sort_unstable();
    assert_eq!(seen, vec![20, 21, 22, 23]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "four one-second calls ran concurrently, not in series"
    );
}

// -- the stateless core ------------------------------------------------

#[tokio::test]
async fn a_request_without_the_required_metadata_is_invalid_params() {
    // FR14, EC-10, and the connection stays usable afterwards because
    // the protocol is stateless and one bad request is not a broken
    // session.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&json!({"jsonrpc": "2.0", "id": 30, "method": "tools/list"}).to_string())
        .await;
    assert_eq!(c.recv().await["error"]["code"], -32602);

    c.send(
        &json!({
            "jsonrpc": "2.0", "id": 31, "method": "tools/list",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": 20260728,
                "io.modelcontextprotocol/clientCapabilities": {}
            }}
        })
        .to_string(),
    )
    .await;
    assert_eq!(c.recv().await["error"]["code"], -32602);

    c.send(&request(32, "tools/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 32, "the session still works");
}

#[tokio::test]
async fn an_unsupported_version_names_what_is_supported() {
    // FR14's version half.
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(
        &json!({
            "jsonrpc": "2.0", "id": 33, "method": "tools/list",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "1900-01-01",
                "io.modelcontextprotocol/clientCapabilities": {}
            }}
        })
        .to_string(),
    )
    .await;
    let r = c.recv().await;
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["supported"], json!([VERSION]));
}

#[tokio::test]
async fn a_line_that_is_not_json_is_a_parse_error_and_not_a_crash() {
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send("{not json").await;
    assert_eq!(c.recv().await["error"]["code"], -32700);
    c.send(&request(34, "tools/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 34);
}
