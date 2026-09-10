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
