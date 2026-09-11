//! The MCP front end: the verb contract as model-controlled tools over
//! stdio (spec 024, `docs/PRD-mcp-front-end.md`).
//!
//! This is a translator and not a second way into the store. It holds no
//! database connection, links no store code, and executes nothing: every
//! call becomes `yeomna call -` on a target, which is why the actor an
//! MCP call leaves in the audit log is the same actor an identical
//! `yeomna call` leaves. That property is the whole reason the transport
//! is stdio with ssh as the remote leg rather than HTTP (D1, D2).
//!
//! **Nothing but valid MCP messages reaches stdout** (D11). The binding
//! requires it, and the hazard here is specific: `yeomna call` prints its
//! envelope to stdout and the CLI tree deliberately puts headers and rows
//! there together, so the child's stdout is captured and never inherited.
//! Logging goes to stderr, which the binding leaves free.

pub mod rpc;
pub mod target;
pub mod tools;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

pub use target::Target;

use rpc::{
    INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR,
    PROTOCOL_VERSION, RpcError, check_meta, error_response, result_response, with_cache_hints,
};

/// How many verb calls may run at once.
///
/// Each one is a process and an appliance session, so an unbounded read
/// loop turns a client's burst into that many Postgres connections or ssh
/// channels. The appliance serializes within a session and not across
/// them, so this is the only place a ceiling exists.
const MAX_CONCURRENT_CALLS: usize = 8;

/// In-flight calls, so `notifications/cancelled` can reach one.
type InFlight = Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>;

/// A key for an id, which JSON-RPC allows to be a string or a number.
fn id_key(id: &Value) -> String {
    id.to_string()
}

/// `server/discover`, which the revision requires every server to answer.
fn discover() -> Value {
    with_cache_hints(json!({
        "supportedVersions": [PROTOCOL_VERSION],
        "capabilities": {
            // False because the list is derived from a compiled-in enum
            // and cannot change while this process runs.
            "tools": {"listChanged": false}
        },
        "instructions":
            "Yeomna's verb contract as tools. Every call is audited on the appliance under \
             the calling user. Graph-scoped verbs take `graph` in their arguments. Verbs that \
             read the session's scope, `query` among them, take their graph and database from \
             the appliance's own config, so `query` with `hybrid` needs a config there naming \
             both.",
    }))
}

/// `tools/list`, in contract order, with R29's exclusion applied.
fn list_tools() -> Value {
    let tools: Vec<Value> = tools::tools().iter().map(tools::Tool::to_json).collect();
    with_cache_hints(json!({ "tools": tools }))
}

/// `tools/call`: build the verb request, run it on the target, and map
/// the outcome per D7.
async fn call_tool(target: &Target, params: Option<&Value>, slots: &Semaphore) -> Response {
    let Some(name) = params.and_then(|p| p.get("name")).and_then(Value::as_str) else {
        return Response::Error(RpcError::new(
            INVALID_PARAMS,
            "params.name is required and names the tool to call",
        ));
    };
    // The exclusion covers dispatch and not only the list (R29). A server
    // that filtered its list and dispatched from the whole enum would
    // ship a tool nobody advertises and anybody can call.
    let known = tools::tools().iter().find(|t| t.name == name);
    let Some(tool) = known else {
        let why = if tools::EXCLUDED.contains(&name) {
            format!(
                "Unknown tool: {name}. It is a verb of this appliance and is deliberately not on \
                 this surface, because MCP tools are model-controlled and the charter reserves \
                 raw statement text to a caller who typed it. Reach it with `yeomna call` instead"
            )
        } else {
            format!("Unknown tool: {name}")
        };
        return Response::Error(RpcError::new(METHOD_NOT_FOUND, why));
    };

    let arguments = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    // The contract does every check, including its refusal of unknown
    // fields. This layer builds the shape and reads none of it.
    let request = json!({"verb": tool.name, "args": arguments});

    // The ceiling is taken here and nowhere else. `tools/list` and
    // `server/discover` are answered from a compiled-in contract with no
    // process behind them, so making them queue behind eight running
    // verbs would be a ceiling on the wrong thing.
    let _permit = slots.acquire().await;
    let mut running = match target::start(target, &request).await {
        Ok(r) => r,
        Err(e) => return Response::Error(RpcError::new(rpc::INTERNAL_ERROR, e)),
    };

    match running.finish().await {
        target::Outcome::Answered(envelope) => Response::Result(json!({
            "content": [{"type": "text", "text": text_of(&envelope)}],
            "structuredContent": envelope,
            "isError": false,
        })),
        // A refusal is a result the model can correct against, never a
        // JSON-RPC error. Yeomna's refusals are written to be read.
        target::Outcome::Refused(envelope) => Response::Result(json!({
            "content": [{"type": "text", "text": text_of(&envelope)}],
            "structuredContent": envelope,
            "isError": true,
        })),
        target::Outcome::Failed(why) => Response::Error(RpcError::new(rpc::INTERNAL_ERROR, why)),
    }
}

/// The text content for an envelope: its error first when it has one, so
/// a refusal reads as a sentence rather than as a field to find, with the
/// whole envelope after it either way.
fn text_of(envelope: &Value) -> String {
    let body = serde_json::to_string_pretty(envelope).unwrap_or_else(|_| envelope.to_string());
    match envelope.get("error").and_then(Value::as_str) {
        Some(e) if !e.is_empty() => format!("{e}\n\n{body}"),
        _ => body,
    }
}

/// What handling a request produced.
enum Response {
    Result(Value),
    Error(RpcError),
    /// Cancelled. The revision says send nothing further for that id.
    Silent,
}

/// Handle one request, metadata checked first.
async fn handle(
    target: &Target,
    method: &str,
    params: Option<&Value>,
    slots: &Semaphore,
) -> Response {
    if let Err(e) = check_meta(params) {
        return Response::Error(e);
    }
    match method {
        "server/discover" => Response::Result(discover()),
        "tools/list" => Response::Result(list_tools()),
        "tools/call" => call_tool(target, params, slots).await,
        other => Response::Error(RpcError::new(
            METHOD_NOT_FOUND,
            format!("Unknown method: {other}"),
        )),
    }
}

/// Serve MCP over a byte stream until end of input.
///
/// On end of input the server stops accepting requests, **finishes and
/// answers what is already in flight**, and exits. It does not cancel
/// that work: abandoning a verb that may already have committed is the
/// client's call to make explicitly through `notifications/cancelled`,
/// never something to infer from a closed pipe (D10, as amended by the
/// build).
pub async fn serve<R, W>(reader: R, writer: W, target: Target) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    // One writer, so concurrent calls cannot interleave a line and so
    // every byte on this stream is a message this code produced. It
    // reports a broken output stream rather than exiting quietly, because
    // a server that keeps spawning destructive verbs it can no longer
    // answer is worse than one that stops.
    let writable = Arc::new(AtomicBool::new(true));
    let pump_flag = Arc::clone(&writable);
    let pump = tokio::spawn(async move {
        let mut w = writer;
        while let Some(v) = rx.recv().await {
            let line = v.to_string();
            debug_assert!(!line.contains('\n'), "a message must be one line");
            if w.write_all(line.as_bytes()).await.is_err() || w.write_all(b"\n").await.is_err() {
                pump_flag.store(false, Ordering::SeqCst);
                break;
            }
            let _ = w.flush().await;
        }
    });

    let in_flight: InFlight = Arc::new(Mutex::new(HashMap::new()));
    // Calls run concurrently, and without a ceiling one client loop turns
    // into one `yeomna call` process and one appliance session per line.
    let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS));
    let target = Arc::new(target);
    let mut lines = BufReader::new(reader).lines();
    let mut tasks: Vec<(Value, tokio::task::JoinHandle<()>)> = Vec::new();

    loop {
        if !writable.load(Ordering::SeqCst) {
            break;
        }
        // Prune finished work so a long session does not accumulate a
        // handle per call. Answering for a handler that died is the
        // supervisor's job and not this loop's, because this loop spends
        // its life blocked on the next line.
        reap(&mut tasks, &tx, false).await;

        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            // A read error ends the session the same way end of input
            // does. Returning here would skip the join below and abandon
            // in-flight work, which is the failure the EOF amendment
            // exists to prevent.
            Err(e) => {
                eprintln!("yeomna-mcp: cannot read input: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(error_response(
                    None,
                    &RpcError::new(PARSE_ERROR, format!("not JSON: {e}")),
                ));
                continue;
            }
        };
        // A batch array or a bare scalar parses but is not a message.
        // Without this it would fall into the notification branch below
        // and be dropped in silence, leaving a client that batched its
        // requests waiting for answers that were never queued.
        if !message.is_object() {
            let _ = tx.send(error_response(
                None,
                &RpcError::new(
                    INVALID_REQUEST,
                    "a message must be a JSON object. This revision has no batch form",
                ),
            ));
            continue;
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();

        // A notification has no id and takes no response.
        let Some(id) = id else {
            if method == "notifications/cancelled"
                && let Some(target_id) = message.pointer("/params/requestId")
                && let Some(stop) = in_flight.lock().await.remove(&id_key(target_id))
            {
                let _ = stop.send(());
            }
            continue;
        };
        if id.is_null() {
            let _ = tx.send(error_response(
                None,
                &RpcError::new(INVALID_REQUEST, "a request id must not be null"),
            ));
            continue;
        }
        if method.is_empty() {
            let _ = tx.send(error_response(
                Some(&id),
                &RpcError::new(INVALID_REQUEST, "a request must name a method"),
            ));
            continue;
        }

        // **A reused id is refused rather than allowed to overwrite.**
        // JSON-RPC forbids reusing an id that has not been answered, and
        // the first build inserted over the old entry, which dropped the
        // running call's cancellation sender. A dropped sender completes
        // its receiver exactly as a real cancellation does, so both calls
        // were killed and neither ever answered.
        let key = id_key(&id);
        let mut live = in_flight.lock().await;
        if live.contains_key(&key) {
            drop(live);
            let _ = tx.send(error_response(
                Some(&id),
                &RpcError::new(
                    INVALID_REQUEST,
                    "this id is already in flight. An id must not be reused until it is answered",
                ),
            ));
            continue;
        }
        let (stop, cancel) = oneshot::channel();
        live.insert(key.clone(), stop);
        drop(live);

        let tx = tx.clone();
        let target = Arc::clone(&target);
        let in_flight = Arc::clone(&in_flight);
        let slots = Arc::clone(&slots);
        let method = method.to_string();
        let params = message.get("params").cloned();
        let task_id = id.clone();
        tasks.push((
            id.clone(),
            tokio::spawn(async move {
                let worker_tx = tx.clone();
                let worker_id = task_id.clone();
                // **The handler runs under a supervisor**, so a panic in
                // it answers the moment it happens. Reaping from the read
                // loop cannot do that: the loop is blocked on the next
                // line, so an id would go unanswered until the client
                // sent something else or closed the stream, and a client
                // waiting on that answer sends nothing.
                let worker = tokio::spawn(async move {
                    // Cancellation covers every method, not only
                    // `tools/call`. Dropping the handler future drops the
                    // child with it, and the command carries
                    // `kill_on_drop`.
                    let response = tokio::select! {
                        // A dropped sender resolves to `Err`, which is
                        // not a cancellation. The pattern disables that
                        // branch so only a real signal wins.
                        Ok(()) = cancel => Response::Silent,
                        r = handle(&target, &method, params.as_ref(), &slots) => r,
                    };
                    match response {
                        Response::Result(r) => {
                            let _ = worker_tx.send(result_response(&worker_id, r));
                        }
                        Response::Error(e) => {
                            let _ = worker_tx.send(error_response(Some(&worker_id), &e));
                        }
                        Response::Silent => {}
                    }
                });
                if let Err(e) = worker.await {
                    let _ = tx.send(error_response(
                        Some(&task_id),
                        &RpcError::new(
                            INTERNAL_ERROR,
                            format!("the server failed while handling this request: {e}"),
                        ),
                    ));
                }
                in_flight.lock().await.remove(&id_key(&task_id));
            }),
        ));
    }

    // End of input. **In-flight calls are finished and answered, not
    // cancelled.** The first build cancelled them, on a reading of "exit
    // promptly when stdin closes" that dogfooding falsified: a client
    // that writes its requests and closes the stream lost every answer
    // whose call was still running, and worse, a verb that had already
    // committed left a NULL audit outcome for work that succeeded.
    // Abandoning committed work is the client's call to make explicitly,
    // through `notifications/cancelled`, and never something to infer
    // from a closed pipe. Prompt exit is still honored: nothing new is
    // accepted, and the binding gives the client SIGTERM and SIGKILL as
    // its backstop if a call runs longer than it will wait.
    reap(&mut tasks, &tx, true).await;
    drop(tx);
    let _ = pump.await;
    Ok(())
}

/// Collect finished handlers, answering for any that died.
///
/// Each handler already answers for itself, panics included, through the
/// supervisor it runs under. This is memory hygiene plus a backstop for
/// the supervisor itself. `drain` is true at end of input, where every
/// remaining task is awaited to completion.
async fn reap(
    tasks: &mut Vec<(Value, tokio::task::JoinHandle<()>)>,
    tx: &mpsc::UnboundedSender<Value>,
    drain: bool,
) {
    let mut keep = Vec::new();
    for (id, handle) in std::mem::take(tasks) {
        if !drain && !handle.is_finished() {
            keep.push((id, handle));
            continue;
        }
        if let Err(e) = handle.await {
            let _ = tx.send(error_response(
                Some(&id),
                &RpcError::new(
                    INTERNAL_ERROR,
                    format!("the server failed while handling this request: {e}"),
                ),
            ));
        }
    }
    *tasks = keep;
}
