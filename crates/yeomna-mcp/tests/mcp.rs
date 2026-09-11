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
use yeomna_mcp::{RemoteConfig, Target, serve, tools};

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

/// Child-spawning tests run one at a time.
///
/// **Not a style choice.** These tests write an executable stand-in and
/// then exec it. When one test is mid-write while another forks, the
/// forked process briefly holds a writable descriptor to the first
/// script, and the later exec fails with `ETXTBSY`, "Text file busy".
/// It reproduced about once in four full runs of this file and surfaced
/// as a different test each time, which is the shape of a harness race
/// rather than a defect in what is being tested. Serializing the writes
/// against the forks closes the window.
fn serial() -> &'static tokio::sync::Mutex<()> {
    static S: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
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
            let _ = serve(server_reads, server_writes, target, RemoteConfig::default()).await;
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

    /// Messages that arrive within a window. Lets a test assert that
    /// something did *not* come back, which a blocking read cannot.
    async fn drain_for(&mut self, ms: u64) -> Vec<Value> {
        let mut seen = Vec::new();
        let deadline = std::time::Duration::from_millis(ms);
        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            let mut line = String::new();
            let left = deadline.saturating_sub(started.elapsed());
            match tokio::time::timeout(left, self.from_server.read_line(&mut line)).await {
                Ok(Ok(n)) if n > 0 => seen.push(serde_json::from_str(&line).unwrap()),
                Ok(_) => break,
                Err(_) => break,
            }
        }
        seen
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(1, "tools/list", json!({}))).await;
    let r = c.recv().await;
    let tools = r["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 41);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // The wire response binds the order too, not only the unit test that
    // builds the list. Contract order is `WIRE_NAMES`, minus R29.
    let want: Vec<&str> = yeomna_verbs::WIRE_NAMES
        .iter()
        .copied()
        .filter(|w| !tools::EXCLUDED.contains(w))
        .collect();
    assert_eq!(
        names, want,
        "tools/list is the contract table, in its order"
    );
    assert!(!names.contains(&"sql"), "sql is on the surface (R29)");
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    assert_eq!(r["result"]["isError"], false, "got {r}");
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    assert_eq!(r["result"]["isError"], false, "got {r}");

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
    let _serial = serial().lock().await;
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
    assert_eq!(r["result"]["isError"], false, "the call completed: {r}");
}

#[tokio::test]
async fn a_large_envelope_arrives_whole() {
    // Review raised stdout truncation. There is none, and this is what
    // says so: stdout is read to the end and kept entire, because it
    // carries the envelope the caller is owed. Only stderr is bounded,
    // and only in what it retains.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let filler = "x".repeat(200_000);
    let envelope = format!(
        r#"{{"success":true,"command":"get","data":{{"body":"{filler}"}},"timestamp":"t"}}"#
    );
    let script_body = format!("cat > /dev/null\nprintf '%s' '{envelope}'\nexit 0");
    let mut c = Client::start(
        Target::local().with_program(
            script(d.path(), "yeomna", &script_body)
                .display()
                .to_string(),
        ),
    );
    c.send(&request(
        90,
        "tools/call",
        json!({"name": "get", "arguments": {"kind": "k", "key": "v"}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false, "got {r}");
    assert_eq!(
        r["result"]["structuredContent"]["data"]["body"]
            .as_str()
            .map(str::len),
        Some(200_000),
        "the envelope survived whole"
    );
}

#[tokio::test]
async fn a_child_that_floods_stderr_does_not_deadlock_the_call() {
    // Found in self-review, not by the spec. Draining stdout to end of
    // input before touching stderr deadlocks a child that fills the
    // stderr pipe buffer: it blocks writing stderr, so it never exits, so
    // stdout never reaches end of input. An ingest that logs is exactly
    // that child, and the symptom is a hang rather than an error.
    let _serial = serial().lock().await;
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
    assert_eq!(r["result"]["isError"], false, "the call completed: {r}");
}

#[tokio::test]
async fn ssh_failing_is_reported_as_the_transport_and_not_as_a_refusal() {
    // FR16, EC-3. `yeomna call` returns only 0, 1, or 2, so 255 is
    // distinguishable, and a caller who cannot tell transport from
    // appliance debugs the wrong machine.
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    assert!(m.contains("not an envelope"), "got {m:?} in {r}");
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
    let _serial = serial().lock().await;
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
    assert_eq!(r["result"]["isError"], false, "got {r}");
}

#[tokio::test]
async fn end_of_input_ends_the_session() {
    // FR11.
    let _serial = serial().lock().await;
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
    // FR10, D10. The first version of this test asserted only that the
    // next message had the later id, which was true whether or not
    // cancellation did anything: the stand-in slept longer than the test
    // ran either way. Deleting the whole cancellation branch left it
    // passing. This one binds the requirement two ways: the child must
    // not reach its own finish line, and the cancelled id must produce
    // no message at all.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let finished = d.path().join("finished");
    let body = format!(
        "cat > /dev/null\nsleep 2\ntouch '{}'\nprintf '%s' '{OK_ENVELOPE}'\nexit 0",
        finished.display()
    );
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        12,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
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
    c.send(&request(13, "tools/list", json!({}))).await;

    // Well past the stand-in's own sleep, so an uncancelled child would
    // have finished and answered inside this window.
    let seen = c.drain_for(3_500).await;
    let ids: Vec<i64> = seen.iter().filter_map(|m| m["id"].as_i64()).collect();
    assert!(ids.contains(&13), "the session kept working: {ids:?}");
    assert!(
        !ids.contains(&12),
        "the cancelled id was answered anyway: {ids:?}"
    );
    assert!(
        !finished.exists(),
        "the child ran to completion, so it was never killed"
    );
}

#[tokio::test]
async fn a_reused_id_is_refused_instead_of_killing_both_calls() {
    // Found in review. The first build inserted over the live entry in
    // the in-flight map, which dropped the running call's cancellation
    // sender. A dropped sender completes its receiver exactly as a real
    // cancellation does, so both calls were killed and the client got
    // nothing at all, forever.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nsleep 1\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&request(
        50,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    c.send(&request(
        50,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;

    let seen = c.drain_for(4_000).await;
    assert_eq!(seen.len(), 2, "both lines were answered: {seen:?}");
    let refused = seen
        .iter()
        .filter(|m| m.get("error").is_some())
        .collect::<Vec<_>>();
    assert_eq!(refused.len(), 1, "exactly one was refused for the reuse");
    assert_eq!(refused[0]["error"]["code"], -32600);
    let answered = seen.iter().filter(|m| m.get("result").is_some()).count();
    assert_eq!(answered, 1, "the first call still ran and answered");
}

#[tokio::test]
async fn a_message_that_is_not_an_object_is_refused_rather_than_dropped() {
    // Found in review. A batch array or a bare scalar parses, carries no
    // `id` and no `method`, and so fell into the notification branch and
    // vanished. A client that batched its requests waited forever.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    for bad in ["[{\"jsonrpc\":\"2.0\"}]", "5", "\"hello\""] {
        c.send(bad).await;
        let r = c.recv().await;
        assert_eq!(r["error"]["code"], -32600, "for {bad}");
    }
    c.send(&request(51, "tools/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 51, "the session still works");
}

#[tokio::test]
async fn answering_from_the_contract_does_not_queue_behind_running_calls() {
    // The concurrency ceiling exists for calls that spawn a process.
    // `tools/list` is answered from a compiled-in enum with nothing
    // behind it, so making it wait for eight running verbs would be a
    // ceiling on the wrong thing.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nsleep 3\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    // Fill every slot and then some.
    for id in 100..110 {
        c.send(&request(
            id,
            "tools/call",
            json!({"name": "status", "arguments": {}}),
        ))
        .await;
    }
    c.send(&request(111, "tools/list", json!({}))).await;
    let started = std::time::Instant::now();
    let r = c.recv().await;
    assert_eq!(r["id"], 111, "the contract answered first: {}", r["id"]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "it waited on the running calls"
    );
}

#[tokio::test]
async fn a_child_that_answers_without_reading_its_input_is_not_a_send_failure() {
    // A verb that refuses early answers and exits without draining its
    // stdin, which breaks the pipe under the write. Reporting that as
    // "cannot send the request" would throw away a good envelope and name
    // the wrong thing.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    // Never reads stdin at all, and exits at once.
    let body = format!("printf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    // A large argument makes the unread write big enough to block and
    // then break, rather than fitting in the pipe buffer unnoticed.
    let big = "x".repeat(500_000);
    c.send(&request(
        112,
        "tools/call",
        json!({"name": "query", "arguments": {"search_text": big}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false, "got {r}");
    assert_eq!(r["result"]["structuredContent"]["data"]["actor"], "todd");
}

#[tokio::test]
async fn an_answer_past_the_ceiling_is_refused_rather_than_truncated() {
    // The embedder PRD's D5 reasoning, applied here: half an envelope is
    // malformed rather than smaller, and handing a model a fragment with
    // nothing saying so is worse than an error.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body =
        "cat > /dev/null\ndd if=/dev/zero bs=1048576 count=17 2>/dev/null | tr '\\0' 'x'\nexit 0";
    let mut c = Client::start(local(d.path(), body));
    c.send(&request(
        113,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    let m = r["error"]["message"].as_str().unwrap_or_default();
    assert!(m.contains("larger than this surface carries"), "got {r}");
    assert!(m.contains("refused rather than truncated"), "got {m:?}");
}

#[tokio::test]
async fn more_calls_than_slots_all_complete() {
    // The concurrency ceiling bounds how many children exist at once. It
    // must not lose or stall the ones beyond it.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    for id in 60..80 {
        c.send(&request(
            id,
            "tools/call",
            json!({"name": "status", "arguments": {}}),
        ))
        .await;
    }
    let mut seen = Vec::new();
    for _ in 0..20 {
        seen.push(c.recv().await["id"].as_i64().unwrap());
    }
    seen.sort_unstable();
    assert_eq!(seen, (60..80).collect::<Vec<_>>());
}

#[tokio::test]
async fn concurrent_calls_each_get_their_own_child() {
    // EC-6. Each call spawns its own `yeomna call`, its own connection,
    // and its own session, so the appliance's per-session serialization
    // is unaffected.
    let _serial = serial().lock().await;
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

// -- the older era, which is how a real client opened -----------------

/// The handshake a client of the previous era sends first.
fn initialize(id: i64, version: &str) -> String {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    })
    .to_string()
}

/// A request with no per-request metadata, which is all the older era sends.
fn legacy_request(id: i64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

#[tokio::test]
async fn a_client_that_opens_with_a_handshake_is_served() {
    // **This is the defect that shipped.** The server required per-request
    // metadata before it looked at the method, so `initialize` died at the
    // door with -32602 and the client reported "failed to connect". The
    // specification's own compatibility matrix names the cell, and says a
    // client of that era has no fall-forward mechanism.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));

    c.send(&initialize(0, "2025-06-18")).await;
    let r = c.recv().await;
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18", "got {r}");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(r["result"]["serverInfo"]["name"], "yeomna-mcp");

    // The notification that closes the handshake wants no answer.
    c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
        .await;

    // And now everything works with no metadata on anything.
    c.send(&legacy_request(1, "tools/list", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 41);

    c.send(&legacy_request(
        2,
        "tools/call",
        json!({"name": "status", "arguments": {}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false, "got {r}");
}

#[tokio::test]
async fn an_extension_block_carrying_only_a_progress_token_is_ordinary() {
    // **Two defects in one sequence, both reported from a live session.**
    // `_meta` is the specification's open extension slot and a
    // `progressToken` lives in it in every era, so reading the block's
    // presence as a promise about this server's own keys refused ordinary
    // client behaviour. Worse, the era was promoted on sight, so that one
    // request flipped an established handshake session to the modern era
    // for the rest of the process and every later request failed too. One
    // progress token poisoned the whole connection.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let body = format!("cat > /dev/null\nprintf '%s' '{OK_ENVELOPE}'\nexit 0");
    let mut c = Client::start(local(d.path(), &body));
    c.send(&initialize(0, "2025-06-18")).await;
    let _ = c.recv().await;

    // The block, carrying a key that belongs to no era in particular.
    c.send(&legacy_request(
        1,
        "tools/list",
        json!({"_meta": {"progressToken": 1}}),
    ))
    .await;
    let r = c.recv().await;
    assert!(
        r.get("error").is_none(),
        "a progress token is not an era: {r}"
    );
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 41);

    // And the session is not poisoned: a plain request still works, which
    // is the half that made this fatal rather than annoying.
    c.send(&legacy_request(2, "tools/list", json!({}))).await;
    let r = c.recv().await;
    assert!(r.get("error").is_none(), "the session survived: {r}");

    // Including a call that reaches the appliance.
    c.send(&legacy_request(
        3,
        "tools/call",
        json!({"name": "status", "arguments": {}, "_meta": {"progressToken": "abc"}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["result"]["isError"], false, "got {r}");
}

#[tokio::test]
async fn the_modern_era_still_wants_its_version_on_every_request() {
    // The fix must not loosen the modern era: the version key is required
    // per request there, and an extension block without it is malformed.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    // A well-formed modern request settles the era.
    c.send(&request(1, "tools/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 1);
    // Then one carrying only a progress token, with no handshake behind
    // it, is still the modern era's malformed case.
    c.send(&legacy_request(
        2,
        "tools/list",
        json!({"_meta": {"progressToken": 9}}),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["error"]["code"], -32602, "got {r}");
}

#[tokio::test]
async fn the_older_era_is_not_sent_fields_from_the_newer_one() {
    // A client validating what it receives should not have to tolerate
    // fields from a revision it did not negotiate. `resultType` and the
    // caching hints are both 2026-07-28.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&initialize(0, "2025-11-25")).await;
    let r = c.recv().await;
    assert!(r["result"].get("resultType").is_none(), "got {r}");
    assert!(r["result"].get("_meta").is_none(), "got {r}");

    c.send(&legacy_request(1, "tools/list", json!({}))).await;
    let r = c.recv().await;
    assert!(r["result"].get("resultType").is_none(), "got {r}");
    assert!(r["result"].get("ttlMs").is_none(), "no caching hints: {r}");
    assert!(r["result"].get("cacheScope").is_none(), "got {r}");
}

#[tokio::test]
async fn an_unknown_handshake_version_gets_a_counter_offer_not_a_refusal() {
    // The older lifecycle says the server answers with a version it does
    // support rather than failing, and lets the client decide.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&initialize(0, "1.0.0")).await;
    let r = c.recv().await;
    assert!(
        r.get("error").is_none(),
        "a counter-offer, not an error: {r}"
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test]
async fn the_newer_era_still_works_and_is_unaffected() {
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(1, "server/discover", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["supportedVersions"], json!([VERSION]));
    assert_eq!(r["result"]["cacheScope"], "public");
}

#[tokio::test]
async fn ping_is_answered_in_both_eras() {
    // Its absence is a hang rather than an error, which is the worst kind
    // of gap in a protocol a client drives.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&request(1, "ping", json!({}))).await;
    assert_eq!(c.recv().await["id"], 1);
    c.send(&initialize(2, "2025-06-18")).await;
    let _ = c.recv().await;
    c.send(&legacy_request(3, "ping", json!({}))).await;
    assert_eq!(c.recv().await["id"], 3);
}

#[tokio::test]
async fn the_metadata_error_names_both_eras_so_a_person_can_act_on_it() {
    // The client that could not connect surfaced exactly this string and
    // nothing else, so it has to say what to do.
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send(&legacy_request(1, "tools/list", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["error"]["code"], -32602);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize handshake"),
        "got {r}"
    );
    assert_eq!(r["error"]["data"]["supportedLegacy"][0], "2025-11-25");
}

// -- the stateless core ------------------------------------------------

#[tokio::test]
async fn a_request_without_the_required_metadata_is_invalid_params() {
    // FR14, EC-10, and the connection stays usable afterwards because
    // the protocol is stateless and one bad request is not a broken
    // session.
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
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
    let _serial = serial().lock().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Client::start(local(d.path(), "exit 0"));
    c.send("{not json").await;
    assert_eq!(c.recv().await["error"]["code"], -32700);
    c.send(&request(34, "tools/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 34);
}
