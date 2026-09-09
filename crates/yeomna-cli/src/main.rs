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

use std::io::Read;
use std::process::ExitCode;

use yeomna_verbs::{Session, Verb, WIRE_NAMES, actor};

/// 0 the verb answered, 1 the verb refused or failed, 2 the caller or
/// the machine was wrong and no call was made.
const OK: u8 = 0;
const VERB_FAILED: u8 = 1;
const USAGE_ERROR: u8 = 2;

const USAGE: &str = "\
yeomna, the sealed knowledge-graph appliance

USAGE:
    yeomna call <json>       run one verb, print its envelope
    yeomna call -            the same, reading the request from stdin
    yeomna verbs             print every wire name in the contract

OPTIONS:
    --graph <name>           scope the session to this graph,
                             overriding the config file's default

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
    let mut positional: Vec<String> = Vec::new();
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--graph" => match rest.next() {
                Some(g) => graph = Some(g.clone()),
                None => return fail("--graph needs a name"),
            },
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::from(OK);
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
            Ok(verb) => run(verb, graph).await,
            Err(code) => code,
        },
        Some(other) => fail(format!("unknown command {other:?}\n\n{USAGE}")),
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

async fn run(verb: Verb, graph: Option<String>) -> ExitCode {
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
    let code = if envelope.success { OK } else { VERB_FAILED };
    match serde_json::to_string_pretty(&envelope) {
        Ok(text) => println!("{text}"),
        // The verb ran and its answer will not serialize, which is this
        // crate's fault and not the caller's, so it does not masquerade
        // as a verb failure.
        Err(e) => return fail(format!("cannot render the envelope: {e}")),
    }
    ExitCode::from(code)
}
