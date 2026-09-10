//! `yeomna`, the appliance's command surface (spec 017).
//!
//! One command reaches the whole contract: `call` takes a verb request
//! as JSON and prints the envelope. That shape is deliberate. The
//! vocabulary is closed and every verb already answers in one envelope,
//! so a per-verb command tree would be a second surface to keep in step
//! with the first, and it waits for Phase 7 where the ergonomics are
//! the point. Here the caller is an agent, and an agent wants the
//! envelope.
//!
//! This binary is thin on purpose. It parses, connects, calls, and
//! prints. Every decision about what a verb means lives in
//! `yeomna-verbs`, and this crate emits no SQL and is not on the lint's
//! allowlist.
//!
//! Embedded mode links the verb layer and opens its own session, which
//! is what makes the graph reachable before the daemon exists. The
//! framed transport lands with Phase 5 behind this same command.

mod config;
mod render;
mod tools;

use std::io::Read;
use std::process::ExitCode;

use serde_json::Value;

use yeomna_verbs::{Session, Verb, WIRE_NAMES, actor};

/// 0 the verb answered, 1 the verb refused or failed, 2 the caller or
/// the machine was wrong and no call was made.
const OK: u8 = 0;
const VERB_FAILED: u8 = 1;
const USAGE_ERROR: u8 = 2;

const USAGE: &str = "\
yeomna, the sealed knowledge-graph appliance

USAGE:
    yeomna <verb...> [--key value]   run a verb by name, print a table
    yeomna call <json>       run one verb, print its envelope
    yeomna call -            the same, reading the request from stdin
    yeomna verbs             print every wire name in the contract
    yeomna tools status      what each analyzer resolves to
    yeomna tools install <analyzer> --from <path>

A verb's command is its wire name with the dots as spaces, so
`graph.traverse` is `yeomna graph traverse`. Arguments are the request's
own field names. `yeomna verbs` lists them all.

    yeomna graph neighbors --graph yeomna_self --key foo --direction in
    yeomna list --kind callable --limit 5

OPTIONS:
    --graph <name>           scope the session to this graph,
                             overriding the config file's default
    --json                   print the envelope verbatim instead of a
                             table. `call` always does this, since its
                             caller is a program
    --daemon                 send the request to yeomnad over its
                             socket instead of linking the verb layer.
                             Same JSON, same envelope, and the actor
                             comes from the kernel either way.

CONFIG:
    YEOMNA_CONFIG names a TOML file, otherwise /etc/yeomna/yeomna.toml
    when it exists, otherwise the built-in defaults.

EXIT:
    0 the verb answered, 1 the verb refused or failed, 2 usage
";

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("yeomna: {message}");
    ExitCode::from(USAGE_ERROR)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut graph: Option<String> = None;
    let mut daemon = false;
    let mut raw_json = false;
    let mut positional: Vec<String> = Vec::new();
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--graph" => match rest.next() {
                Some(g) => {
                    graph = Some(g.clone());
                    // Also offered to the verb, because a request may
                    // carry `graph` as a field. Which of the two it
                    // means is the contract's to say, not a table kept
                    // here (see `verb_from_path`).
                    flags.push(("graph".to_string(), Some(g.clone())));
                }
                None => return fail("--graph needs a name"),
            },
            "--daemon" => daemon = true,
            "--json" => raw_json = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::from(OK);
            }
            // Any other `--key` belongs to the verb being called, and
            // its value is the next argument unless the next argument is
            // another flag, in which case this one is a bare true (EC-2).
            other if other.starts_with("--") => {
                let key = other.trim_start_matches('-').to_string();
                let value = match rest.clone().next() {
                    Some(v) if !v.starts_with("--") => rest.next().cloned(),
                    _ => None,
                };
                flags.push((key, value));
            }
            other => positional.push(other.to_string()),
        }
    }

    match positional.first().map(String::as_str) {
        Some("verbs") => {
            for name in WIRE_NAMES {
                println!("{name}");
            }
            ExitCode::from(OK)
        }
        Some("call") => match read_request(&positional) {
            Ok(verb) => {
                if daemon {
                    // `call` is the program's surface, so it prints the
                    // envelope whatever else was asked for.
                    through_daemon(verb, true).await
                } else {
                    embedded(verb, graph, true).await
                }
            }
            Err(code) => code,
        },
        Some("tools") => tools_command(&positional, &flags),
        // Everything else is a verb path: the wire name with its dots as
        // spaces (D10). The tree is derived from the contract rather
        // than written beside it, so a verb added to `Verb` gets its
        // command the same day and the two cannot drift.
        Some(_) => match verb_from_path(&positional, &flags) {
            Ok(verb) => {
                if daemon {
                    through_daemon(verb, raw_json).await
                } else {
                    embedded(verb, graph, raw_json).await
                }
            }
            Err(code) => code,
        },
        None => {
            eprint!("{USAGE}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// The request, from the argument or from stdin. The closed enum does
/// the validating: an unknown verb name or an unknown field inside a
/// request is a deserialization error naming the stranger, which is
/// what makes a typo a refusal rather than a surprise.
fn read_request(positional: &[String]) -> Result<Verb, ExitCode> {
    let Some(source) = positional.get(1) else {
        return Err(fail(format!("call needs a request\n\n{USAGE}")));
    };
    let text = if source == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            return Err(fail(format!("cannot read stdin: {e}")));
        }
        buf
    } else {
        source.clone()
    };
    if text.trim().is_empty() {
        return Err(fail("the request is empty"));
    }
    serde_json::from_str(&text).map_err(|e| fail(format!("not a verb request: {e}")))
}

async fn embedded(verb: Verb, graph: Option<String>, raw_json: bool) -> ExitCode {
    let config = match config::load() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let client = match yeomna_store::connect(
        &config.socket_dir,
        config.port,
        "yeomna_app",
        &config.database,
    )
    .await
    {
        Ok(c) => c,
        // EC-1 and EC-4: the store is not answering where the config
        // said it would, and the message names where that was.
        Err(e) => {
            return fail(format!(
                "cannot reach the store at {} port {}: {e}",
                config.socket_dir, config.port
            ));
        }
    };

    let mut session = Session::new(client, actor::from_kernel())
        .with_endpoint(config.socket_dir.clone(), config.port);
    if let Some(g) = graph.or(config.graph) {
        session = session.with_graph(g);
    }

    let envelope = session.call(&verb).await;
    emit(&envelope, raw_json)
}

/// One envelope out, either verbatim for a program or as text for a
/// person. Both views render the same envelope, so they cannot disagree
/// about the answer, and both go entirely to stdout.
fn emit(envelope: &yeomna_verbs::Envelope, raw_json: bool) -> ExitCode {
    let code = if envelope.success { OK } else { VERB_FAILED };
    if raw_json {
        match serde_json::to_string_pretty(envelope) {
            Ok(text) => println!("{text}"),
            // The verb ran and its answer will not serialize, which is
            // this crate's fault and not the caller's, so it does not
            // masquerade as a verb failure.
            Err(e) => return fail(format!("cannot render the envelope: {e}")),
        }
    } else {
        print!("{}", render::envelope(envelope));
    }
    ExitCode::from(code)
}

/// FR8: the same request over the daemon's socket. The transport is a
/// deployment choice and the contract does not notice: the caller sends
/// the JSON it would have sent, and the daemon names it from the kernel
/// exactly as embedded mode does, so `--graph` belongs to the daemon's
/// configuration rather than to a frame.
async fn through_daemon(verb: Verb, raw_json: bool) -> ExitCode {
    use tokio::net::UnixStream;
    use yeomna_verbs::frame;

    let config = match config::load() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let path = config.daemon_socket();
    let mut stream = match UnixStream::connect(&path).await {
        Ok(s) => s,
        Err(e) => return fail(format!("cannot reach the daemon at {path}: {e}")),
    };
    let body = match serde_json::to_vec(&verb) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot render the request: {e}")),
    };
    if let Err(e) = frame::write(&mut stream, &body).await {
        return fail(format!("cannot send the request: {e}"));
    }
    let response = match frame::read(&mut stream).await {
        Ok(r) => r,
        Err(e) => return fail(format!("no answer from the daemon: {e}")),
    };
    let envelope: yeomna_verbs::Envelope = match serde_json::from_slice(&response) {
        Ok(e) => e,
        Err(e) => return fail(format!("the daemon's answer is not an envelope: {e}")),
    };
    emit(&envelope, raw_json)
}

/// A verb path plus its flags, turned into a `Verb`.
///
/// No per-verb parsing code exists, and that is the design. The path is
/// a wire name with spaces for dots (D10), the flags become a JSON
/// object, and the closed enum does every check: an unknown verb name,
/// an unknown field, a missing required field, and a value of the wrong
/// type are all deserialization errors naming what was wrong. A verb
/// added to the contract is therefore reachable immediately, which is
/// stronger than a hand-written tree that has to be remembered.
fn verb_from_path(
    positional: &[String],
    flags: &[(String, Option<String>)],
) -> Result<Verb, ExitCode> {
    // Longest match first, so `graph.shortest-path` wins over `graph`.
    let mut name: Option<&str> = None;
    for candidate in WIRE_NAMES {
        let parts: Vec<&str> = candidate.split('.').collect();
        if parts.len() <= positional.len()
            && parts
                .iter()
                .zip(positional)
                .all(|(p, given)| *p == given.as_str())
            && parts.len() == positional.len()
        {
            name = Some(candidate);
            break;
        }
    }
    let Some(name) = name else {
        return Err(fail(format!(
            "unknown command {:?}{}\n\nRun `yeomna verbs` for the whole list.",
            positional.join(" "),
            nearest(positional)
        )));
    };

    let mut args = serde_json::Map::new();
    for (key, value) in flags {
        args.insert(key.clone(), json_value(value.as_deref()));
    }
    let build = |args: &serde_json::Map<String, Value>| {
        serde_json::from_value::<Verb>(
            serde_json::json!({"verb": name, "args": Value::Object(args.clone())}),
        )
    };
    match build(&args) {
        Ok(verb) => Ok(verb),
        Err(e) if e.to_string().contains("unknown field `graph`") => {
            // The verb takes its graph from the session rather than from
            // a field, so `--graph` scoped the session and does not
            // belong in the args. The contract said which, which is why
            // no list of graph-carrying verbs is kept here.
            args.remove("graph");
            build(&args).map_err(|e| {
                fail(format!(
                    "{}: {e}\n\nThe arguments are the request's own field names.",
                    positional.join(" ")
                ))
            })
        }
        Err(e) => Err(fail(format!(
            "{}: {e}\n\nThe arguments are the request's own field names.",
            positional.join(" ")
        ))),
    }
}

/// A flag's value, typed. Parsed as JSON first so numbers, booleans, and
/// arrays arrive as themselves, and falling back to a string when that
/// fails, which is what a bare word is. A flag with no value is `true`,
/// which is what a flag means (EC-2).
fn json_value(raw: Option<&str>) -> Value {
    match raw {
        None => Value::Bool(true),
        Some(text) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
        }
    }
}

/// Commands sharing the first word the caller typed, so a near miss gets
/// a short list rather than the whole contract (EC-1).
fn nearest(positional: &[String]) -> String {
    let Some(first) = positional.first() else {
        return String::new();
    };
    let close: Vec<String> = WIRE_NAMES
        .iter()
        .filter(|n| n.starts_with(first.as_str()) || n.split('.').next() == Some(first.as_str()))
        .map(|n| format!("  yeomna {}", n.replace('.', " ")))
        .collect();
    if close.is_empty() {
        String::new()
    } else {
        format!("\n\nDid you mean:\n{}", close.join("\n"))
    }
}

/// `yeomna tools ...` (H8). Not a verb: it touches the operator's
/// filesystem and the analyzers the ingest spawns, never the store.
fn tools_command(positional: &[String], flags: &[(String, Option<String>)]) -> ExitCode {
    match positional.get(1).map(String::as_str) {
        Some("status") => {
            print!("{}", tools::status());
            ExitCode::from(OK)
        }
        Some("install") => {
            let Some(analyzer) = positional.get(2) else {
                return fail(format!(
                    "tools install needs an analyzer: {}",
                    tools::known().join(", ")
                ));
            };
            let from = flags
                .iter()
                .find(|(k, _)| k == "from")
                .and_then(|(_, v)| v.clone());
            match tools::install(analyzer, from.as_deref()) {
                Ok(said) => {
                    print!("{said}");
                    ExitCode::from(OK)
                }
                Err(e) => fail(e),
            }
        }
        other => fail(format!(
            "tools takes status or install, not {:?}",
            other.unwrap_or("nothing")
        )),
    }
}
