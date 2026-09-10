//! The command surface end to end (spec 017): the real binary, real
//! arguments, real exit codes.
//!
//! Running the binary rather than calling a function is the point. The
//! exit code and the stream a message lands on are the contract a
//! caller scripts against, and neither is visible from inside.

use std::process::{Command, Stdio};

use tempfile::TempDir;

const PORT: u16 = 5433;
const BIN: &str = env!("CARGO_BIN_EXE_yeomna");

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

/// A config naming the dev cluster, so the tests do not depend on
/// whether this machine has `/etc/yeomna/yeomna.toml`.
fn config_for(dir: &str, graph: Option<&str>) -> (TempDir, String) {
    let d = TempDir::new().unwrap();
    let path = d.path().join("yeomna.toml");
    let mut text = format!("socket_dir = \"{dir}\"\nport = {PORT}\ndatabase = \"yeomna\"\n");
    if let Some(g) = graph {
        text.push_str(&format!("graph = \"{g}\"\n"));
    }
    std::fs::write(&path, text).unwrap();
    let p = path.to_string_lossy().to_string();
    (d, p)
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the binary with a config and no inherited environment surprises.
fn run(config: Option<&str>, args: &[&str], stdin: Option<&str>) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_remove("YEOMNA_CONFIG")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(c) = config {
        cmd.env("YEOMNA_CONFIG", c);
    }
    let mut child = cmd.spawn().expect("the binary runs");
    if let Some(text) = stdin {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().expect("the process exited"),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// FR6: the vocabulary is discoverable, and it is the contract's own
/// list rather than a copy this crate keeps.
#[test]
fn verbs_prints_the_contract() {
    let r = run(None, &["verbs"], None);
    assert_eq!(r.code, 0);
    let printed: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(printed, yeomna_verbs::WIRE_NAMES.to_vec());
    assert_eq!(printed.len(), 42);
}

/// FR3 and EC-3, EC-5: nothing reaches the store, and the message says
/// what was wrong.
#[test]
fn a_malformed_request_is_a_usage_error_and_makes_no_call() {
    for (args, stdin, expect) in [
        (vec!["call"], None, "needs a request"),
        (vec!["call", "   "], None, "empty"),
        (vec!["call", "{not json"], None, "not a verb request"),
        (
            vec!["call", r#"{"verb":"no-such-verb","args":{}}"#],
            None,
            "not a verb request",
        ),
        (
            // The contract denies unknown fields at both levels, so a
            // smuggled actor dies here rather than reaching a session.
            vec!["call", r#"{"verb":"status","args":{},"actor":"root"}"#],
            None,
            "not a verb request",
        ),
        (vec!["call", "-"], Some(""), "empty"),
    ] {
        let r = run(None, &args, stdin);
        assert_eq!(
            r.code,
            2,
            "{args:?} must be a usage error: {r:?}",
            r = r.stderr
        );
        assert!(
            r.stderr.contains(expect),
            "{args:?} should say {expect:?}, said {:?}",
            r.stderr
        );
        assert!(r.stdout.is_empty(), "{args:?} printed to stdout");
    }
}

#[test]
fn an_unknown_command_and_help_answer_without_a_cluster() {
    let r = run(None, &["nonsense"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("unknown command"));
    let r = run(None, &["--help"], None);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("yeomna call <json>"));
    let r = run(None, &[], None);
    assert_eq!(r.code, 2, "no arguments is a usage error");
}

/// FR4: a config that exists and will not parse is an error naming the
/// file, never a silent fallback to the defaults.
#[test]
fn a_malformed_config_names_the_file_rather_than_falling_back() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("broken.toml");
    std::fs::write(&path, "port = \"not a number\"\n").unwrap();
    let r = run(
        Some(&path.to_string_lossy()),
        &["call", r#"{"verb":"status","args":{}}"#],
        None,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("cannot parse"), "{}", r.stderr);
    assert!(r.stderr.contains("broken.toml"), "{}", r.stderr);
}

/// EC-1 and EC-4: the store is not where the config said, and the
/// message says where that was.
#[test]
fn an_unreachable_store_names_the_path_it_tried() {
    let (_d, config) = config_for("/nonexistent/socket/dir", None);
    let r = run(
        Some(&config),
        &["call", r#"{"verb":"status","args":{}}"#],
        None,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("/nonexistent/socket/dir"), "{}", r.stderr);
    assert!(r.stderr.contains("cannot reach the store"), "{}", r.stderr);
}

/// FR1 and FR2: the envelope is the answer either way, and the exit
/// code says which way.
#[test]
fn a_call_answers_in_an_envelope_from_the_argument_and_from_stdin() {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let (_d, config) = config_for(&dir, None);
    let request = r#"{"verb":"status","args":{}}"#;

    for stdin in [None, Some(request)] {
        let args = if stdin.is_some() {
            vec!["call", "-"]
        } else {
            vec!["call", request]
        };
        let r = run(Some(&config), &args, stdin);
        assert_eq!(r.code, 0, "{}", r.stderr);
        let env: serde_json::Value =
            serde_json::from_str(&r.stdout).expect("stdout is one envelope");
        assert_eq!(env["success"], true);
        assert_eq!(env["command"], "status");
        assert_eq!(env["data"]["role"], "yeomna_app");
        assert!(r.stderr.is_empty(), "{}", r.stderr);
    }

    // FR2: a miss is an envelope too, with exit 1.
    let r = run(
        Some(&config),
        &[
            "call",
            r#"{"verb":"get","args":{"kind":"document","key":"absent"}}"#,
        ],
        None,
    );
    assert_eq!(r.code, 1);
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(env["success"], false);
    assert!(env["error"].as_str().unwrap().starts_with("not-found"));
}

/// EC-6: a verb this phase refuses reaches the caller as an answer,
/// which is what the taxonomy is for.
///
/// `graph.materialize` rather than a phased verb: it refuses until a
/// consumer defines what materialization means (R14), so this guard
/// does not need repointing every time a phase lands. Spec 019
/// implementing `ingest` is what taught that lesson.
#[test]
fn an_unimplemented_verb_refuses_by_name() {
    let Some(dir) = socket_dir() else { return };
    let (_d, config) = config_for(&dir, None);
    let r = run(
        Some(&config),
        &[
            "call",
            r#"{"verb":"graph.materialize","args":{"graph":"g"}}"#,
        ],
        None,
    );
    assert_eq!(r.code, 1);
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    let error = env["error"].as_str().unwrap();
    assert!(error.starts_with("unimplemented"), "{error}");
    assert!(
        error.contains("consumer that defines"),
        "it names what it waits for: {error}"
    );
}

/// EC-2: the CLI does not second-guess the contract. A verb needing a
/// session graph with none configured gets the verb's own refusal, and
/// `--graph` is what supplies one.
#[test]
fn the_session_graph_comes_from_the_flag_or_the_config() {
    let Some(dir) = socket_dir() else { return };
    let (_d, config) = config_for(&dir, None);
    let unscoped = run(
        Some(&config),
        &[
            "call",
            r#"{"verb":"codebase.stats","args":{"graph":"cli_absent_graph"}}"#,
        ],
        None,
    );
    assert_eq!(unscoped.code, 1, "{}", unscoped.stderr);

    // The flag reaches the session: status reports what it was scoped to.
    let scoped = run(
        Some(&config),
        &[
            "--graph",
            "cli_scoped",
            "call",
            r#"{"verb":"status","args":{}}"#,
        ],
        None,
    );
    assert_eq!(scoped.code, 0, "{}", scoped.stderr);
    let env: serde_json::Value = serde_json::from_str(&scoped.stdout).unwrap();
    assert_eq!(env["data"]["session_graph"], "cli_scoped");

    // And so does the config's default, when no flag overrides it.
    let (_d2, with_graph) = config_for(&dir, Some("cli_from_config"));
    let r = run(
        Some(&with_graph),
        &["call", r#"{"verb":"status","args":{}}"#],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(env["data"]["session_graph"], "cli_from_config");

    // The flag wins over the config, which is what an override is.
    let r = run(
        Some(&with_graph),
        &[
            "--graph",
            "cli_flag_wins",
            "call",
            r#"{"verb":"status","args":{}}"#,
        ],
        None,
    );
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(env["data"]["session_graph"], "cli_flag_wins");
}

/// FR5, the half a caller can see from outside: a hostile environment
/// does not stop the call and does not name the caller. That the row
/// carries the kernel's actor is proven where the audit log may be
/// read, in `yeomna-verbs`, since this crate emits no SQL.
#[test]
fn a_hostile_environment_does_not_change_who_is_calling() {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let (_d, config) = config_for(&dir, None);
    let out = Command::new(BIN)
        .args(["call", r#"{"verb":"health","args":{}}"#])
        .env("YEOMNA_CONFIG", &config)
        .env("USER", "impostor")
        .env("LOGNAME", "impostor")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(
        yeomna_verbs::actor::from_kernel(),
        "impostor",
        "the environment does not name the caller"
    );
}

/// FR8 (spec 018): the same JSON over the daemon's socket, the same
/// envelope back. The transport is a deployment choice and the contract
/// does not notice.
#[tokio::test]
async fn the_daemon_transport_answers_like_the_embedded_one() {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let sockets = TempDir::new().unwrap();
    let daemon_socket = sockets.path().join("yeomna.sock");
    let listener = yeomna_daemon::server::bind(&daemon_socket).expect("binds");
    let task = tokio::spawn(yeomna_daemon::server::serve(
        listener,
        yeomna_daemon::server::Settings {
            socket_dir: dir.clone(),
            port: PORT,
            database: "yeomna".to_string(),
            graph: None,
            embedder_socket: sockets.path().join("embedder.sock").display().to_string(),
        },
    ));

    let d = TempDir::new().unwrap();
    let config_path = d.path().join("yeomna.toml");
    std::fs::write(
        &config_path,
        format!(
            "socket_dir = \"{dir}\"\nport = {PORT}\ndatabase = \"yeomna\"\nsocket_path = \"{}\"\n",
            daemon_socket.display()
        ),
    )
    .unwrap();
    let config = config_path.to_string_lossy().to_string();
    let request = r#"{"verb":"status","args":{}}"#;

    // The child runs on the blocking pool. Waiting on a process from a
    // worker thread would occupy the runtime the daemon task needs, and
    // the two would wait on each other.
    let framed = {
        let c = config.clone();
        tokio::task::spawn_blocking(move || run(Some(&c), &["--daemon", "call", request], None))
            .await
            .unwrap()
    };
    let embedded = {
        let c = config.clone();
        tokio::task::spawn_blocking(move || run(Some(&c), &["call", request], None))
            .await
            .unwrap()
    };
    task.abort();

    assert_eq!(framed.code, 0, "{}", framed.stderr);
    assert_eq!(embedded.code, 0, "{}", embedded.stderr);
    let a: serde_json::Value = serde_json::from_str(&framed.stdout).unwrap();
    let b: serde_json::Value = serde_json::from_str(&embedded.stdout).unwrap();
    assert_eq!(a["command"], b["command"]);
    assert_eq!(a["data"]["role"], b["data"]["role"]);
    assert_eq!(
        a["data"]["actor"], b["data"]["actor"],
        "both transports name the caller from the kernel"
    );
}

/// The daemon is not running, and the message says where it looked.
#[test]
fn an_absent_daemon_names_the_socket_it_tried() {
    let (_d, config) = config_for("/tmp", None);
    let r = run(
        Some(&config),
        &["--daemon", "call", r#"{"verb":"status","args":{}}"#],
        None,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("cannot reach the daemon"), "{}", r.stderr);
    assert!(r.stderr.contains("/tmp/yeomna.sock"), "{}", r.stderr);
}

/// FR8, the census inverted. PR #19's census asserted that every command
/// reports its hole. This asserts that every wire name in the contract
/// is reachable as a subcommand, which is the claim that matters once
/// the holes are filled.
///
/// Reachability is tested by whether the path resolves, not by whether
/// the verb succeeds: a resolved path fails on a missing field or on an
/// absent store, and only an unresolved one says "unknown command". So
/// this runs without a cluster and still proves the tree covers the
/// contract.
#[test]
fn every_wire_name_is_reachable_as_a_subcommand() {
    let mut unreachable = Vec::new();
    for name in yeomna_verbs::WIRE_NAMES {
        let path: Vec<&str> = name.split('.').collect();
        let r = run(None, &path, None);
        if r.stderr.contains("unknown command") {
            unreachable.push(name);
        }
    }
    assert!(
        unreachable.is_empty(),
        "the tree does not cover the contract: {unreachable:?}"
    );
    // And the count is the contract's, not a number kept here (PR #19's
    // third do-not-repeat: counts come from the contract).
    assert_eq!(yeomna_verbs::WIRE_NAMES.len(), 42);
}

/// FR2 and FR3: arguments arrive typed, and the contract is what says a
/// name or a type was wrong.
#[test]
fn arguments_are_typed_and_the_contract_checks_them() {
    // A misspelled field: the contract denies unknown fields, so the
    // message names the stranger.
    let r = run(None, &["list", "--kidn", "callable"], None);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("kidn"), "{}", r.stderr);

    // A missing required field: serde says which.
    let r = run(None, &["graph", "neighbors"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("missing field"), "{}", r.stderr);

    // A wrong type: `--limit` is a number, so a word is refused by the
    // contract rather than coerced here.
    let r = run(None, &["list", "--limit", "many"], None);
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("limit") || r.stderr.contains("invalid type"),
        "{}",
        r.stderr
    );
}

/// EC-1: a near miss gets the neighbours, not the whole contract.
#[test]
fn a_near_miss_suggests_rather_than_dumping_the_tree() {
    let r = run(None, &["graph", "nieghbors"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("unknown command"), "{}", r.stderr);
    assert!(r.stderr.contains("Did you mean"), "{}", r.stderr);
    assert!(r.stderr.contains("graph neighbors"), "{}", r.stderr);
    assert!(
        !r.stderr.contains("codebase.ingest") && !r.stderr.contains("codebase ingest"),
        "unrelated commands are not listed: {}",
        r.stderr
    );
}

/// A misspelled root, not only a misspelled leaf.
///
/// The first cut matched a root by prefix or equality, so `grph` and
/// `garph` produced no suggestion at all and EC-1's promise held only for
/// callers who spelled the first word correctly. Edit distance is what
/// closes that, and a transposition costs two, so the budget has to admit
/// two on a five-letter root.
#[test]
fn a_misspelled_root_still_gets_its_neighbours() {
    for typo in ["grph", "garph", "codbase", "datbase"] {
        let r = run(None, &[typo, "list"], None);
        assert_eq!(r.code, 2, "{typo}: {}", r.stderr);
        assert!(
            r.stderr.contains("Did you mean"),
            "{typo} suggested nothing: {}",
            r.stderr
        );
    }
    // And a root that resembles nothing gets the whole-list pointer rather
    // than a list of everything.
    let r = run(None, &["zzzzzzzz", "list"], None);
    assert_eq!(r.code, 2);
    assert!(!r.stderr.contains("Did you mean"), "{}", r.stderr);
    assert!(r.stderr.contains("yeomna verbs"), "{}", r.stderr);
}

/// FR4 and FR5: a table by default, the envelope with `--json`, both to
/// stdout, and the exit code the same either way.
#[test]
fn the_tree_renders_a_table_and_json_on_request() {
    let Some(dir) = socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let (_d, config) = config_for(&dir, None);

    let table = run(Some(&config), &["status"], None);
    assert_eq!(table.code, 0, "{}", table.stderr);
    assert!(table.stderr.is_empty(), "everything went to stdout");
    assert!(table.stdout.contains("store"), "{}", table.stdout);
    assert!(
        !table.stdout.starts_with('{'),
        "the default is a table, not JSON: {}",
        table.stdout
    );

    let raw = run(Some(&config), &["status", "--json"], None);
    assert_eq!(raw.code, 0);
    let env: serde_json::Value = serde_json::from_str(&raw.stdout).expect("an envelope");
    assert_eq!(env["success"], true);

    // FR5: a refusal renders and exits 1 through the tree as through
    // `call` (EC-6).
    let refused = run(Some(&config), &["embed", "text", "--text", "hello"], None);
    assert_eq!(refused.code, 1, "{}", refused.stderr);
    assert!(
        refused.stdout.contains("unimplemented"),
        "{}",
        refused.stdout
    );
    assert!(
        refused.stdout.contains("H4"),
        "it names its hole: {}",
        refused.stdout
    );
}

/// A table has its header on the same stream as its rows, which is the
/// second of PR #19's do-not-repeat items and the one a redirect would
/// have exposed.
#[test]
fn a_table_keeps_its_header_with_its_rows() {
    let Some(dir) = socket_dir() else { return };
    let (_d, config) = config_for(&dir, None);
    // `query` returns an array of like-shaped objects, which is the shape
    // that renders as a table. `codebase stats` returns a map and renders
    // as indented pairs, so it never exercised the table path this test is
    // named for.
    let r = run(
        Some(&config),
        &[
            "--graph",
            "yeomna_self",
            "query",
            "--search_text",
            "recursive parser",
            "--limit",
            "3",
        ],
        None,
    );
    if !r.stdout.contains("hits:") || r.stdout.contains("hits: none") {
        eprintln!("SKIP: no yeomna_self graph with matching chunks on this cluster");
        return;
    }
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.is_empty(), "nothing went to stderr: {}", r.stderr);
    // Asserting stderr is empty is not enough: it also passes when the
    // renderer omits the header entirely, which is the failure this test
    // exists to catch. The header, its rule, and a row have to be on
    // stdout together and in that order.
    let lines: Vec<&str> = r
        .stdout
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("graph "))
        .collect();
    assert!(
        lines.len() >= 3,
        "the table's header is on stdout with its rows: {}",
        r.stdout
    );
    assert!(
        lines[0].contains("key") && lines[0].contains("rank"),
        "header: {:?}",
        lines[0]
    );
    assert!(
        lines[1].trim_start().starts_with("----"),
        "the rule under the header: {:?}",
        lines[1]
    );
    assert!(
        !lines[2].trim().is_empty(),
        "and at least one row after it: {:?}",
        lines[2]
    );
}

/// FR6 and FR7: H8's two commands, through the binary.
#[test]
fn tools_reports_and_refuses_to_reach_the_network() {
    let r = run(None, &["tools", "status"], None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("analyzer"), "{}", r.stdout);
    assert!(r.stdout.contains("rust-analyzer"), "{}", r.stdout);
    assert!(r.stdout.contains("managed tools directory"), "{}", r.stdout);

    // R24: no source, no install, and the message says why.
    let r = run(None, &["tools", "install", "rust-analyzer"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--from"), "{}", r.stderr);
    assert!(r.stderr.contains("section 5"), "{}", r.stderr);

    let r = run(None, &["tools", "install"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("needs an analyzer"), "{}", r.stderr);

    let r = run(None, &["tools", "nonsense"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("status or install"), "{}", r.stderr);
}

/// `--graph` means one thing to a person, and the contract decides
/// whether it is a request field or the session's scope.
#[test]
fn one_graph_flag_serves_both_kinds_of_verb() {
    let Some(dir) = socket_dir() else { return };
    let (_d, config) = config_for(&dir, None);

    // `count` takes its graph from the session.
    let r = run(
        Some(&config),
        &[
            "--graph",
            "yeomna_self",
            "count",
            "--kind",
            "file",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(env["data"]["graph"], "yeomna_self");

    // `codebase.stats` carries it as a field, and the same flag reaches
    // it, which is the collision the contract resolves.
    let r = run(
        Some(&config),
        &["--graph", "yeomna_self", "codebase", "stats", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let env: serde_json::Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(env["data"]["graph"], "yeomna_self");
}
