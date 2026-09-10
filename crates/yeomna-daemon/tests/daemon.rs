//! The daemon end to end (spec 018): a real socket, real frames, and a
//! real peer whose uid the kernel supplies.
//!
//! The server is started in-process on a temporary socket rather than
//! through systemd, because what is under test is the listener and the
//! connection loop, and the unit is a deployment artifact reviewed by
//! reading it.

use std::path::PathBuf;

use tempfile::TempDir;
use tokio::net::UnixStream;
use yeomna_verbs::frame;

const PORT: u16 = 5433;

fn store_socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

/// A daemon on its own socket, serving until the test drops it.
struct Daemon {
    _dir: TempDir,
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn start(store_dir: &str, graph: Option<&str>) -> Daemon {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("yeomna.sock");
    let listener = yeomna_daemon::server::bind(&path).expect("binds");
    let settings = yeomna_daemon::server::Settings {
        socket_dir: store_dir.to_string(),
        port: PORT,
        database: "yeomna".to_string(),
        graph: graph.map(str::to_string),
        // No embedder in these tests. A session still opens, because it
        // connects per call rather than holding a client, so only the
        // verbs that need a vector notice.
        embedder_socket: dir.path().join("embedder.sock").display().to_string(),
    };
    let task = tokio::spawn(yeomna_daemon::server::serve(listener, settings));
    Daemon {
        _dir: dir,
        path,
        task,
    }
}

/// One request, one envelope, on a fresh connection.
async fn ask(path: &PathBuf, request: &str) -> serde_json::Value {
    let mut stream = UnixStream::connect(path).await.expect("the daemon accepts");
    frame::write(&mut stream, request.as_bytes()).await.unwrap();
    let body = frame::read(&mut stream).await.expect("an answer");
    serde_json::from_slice(&body).expect("the answer is an envelope")
}

/// FR1 and FR2: a framed client gets the envelope, and the appliance
/// names the peer the kernel identified. `status` reports the actor for
/// exactly this reason, so the claim is checked through the contract
/// rather than by reading the audit log, which this crate may not do
/// and which no verb reads anyway.
#[tokio::test]
async fn a_framed_client_is_answered_and_named_by_the_kernel() {
    let Some(store) = store_socket_dir() else {
        eprintln!("SKIP: no cluster socket");
        return;
    };
    let d = start(&store, None).await;

    let env = ask(&d.path, r#"{"verb":"status","args":{}}"#).await;
    assert_eq!(env["success"], true, "{env}");
    assert_eq!(env["command"], "status");
    assert_eq!(env["data"]["role"], "yeomna_app");
    assert_eq!(
        env["data"]["actor"],
        yeomna_verbs::actor::from_kernel(),
        "the peer's uid named the session, not anything in the frame"
    );
}

/// FR6: one connection, many requests, in order.
#[tokio::test]
async fn one_connection_carries_many_requests() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, None).await;
    let mut stream = UnixStream::connect(&d.path).await.unwrap();
    for verb in ["status", "health", "schema.version"] {
        let request = format!(r#"{{"verb":"{verb}","args":{{}}}}"#);
        frame::write(&mut stream, request.as_bytes()).await.unwrap();
        let body = frame::read(&mut stream).await.unwrap();
        let env: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(env["command"], verb, "answers arrive in order");
        assert_eq!(env["success"], true);
    }
}

/// FR5: a caller that framed correctly gets an answer in the shape it
/// expects, even when what it framed was not a verb, and the connection
/// stays open for the next request.
#[tokio::test]
async fn malformed_json_is_answered_rather_than_hung_up_on() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, None).await;
    let mut stream = UnixStream::connect(&d.path).await.unwrap();

    frame::write(&mut stream, b"{not json").await.unwrap();
    let body = frame::read(&mut stream)
        .await
        .expect("an answer, not a close");
    let env: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(env["success"], false);
    assert!(
        env["error"].as_str().unwrap().starts_with("invalid-args"),
        "{env}"
    );

    // Still open: the mistake was the caller's and it may try again.
    frame::write(&mut stream, br#"{"verb":"health","args":{}}"#)
        .await
        .unwrap();
    let body = frame::read(&mut stream).await.unwrap();
    let env: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(env["success"], true, "the connection survived");
}

/// FR4: an oversized claim is refused before anything of that size is
/// allocated, and the connection closes rather than being fed.
#[tokio::test]
async fn an_oversized_frame_closes_the_connection() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, None).await;
    let mut stream = UnixStream::connect(&d.path).await.unwrap();
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(&(frame::MAX_FRAME + 1).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(b"a few real bytes").await.unwrap();
    stream.flush().await.unwrap();
    // No answer comes, and the read ends rather than blocking forever.
    assert!(
        frame::read(&mut stream).await.is_err(),
        "the daemon refused and closed"
    );
}

/// EC-1: connect, say nothing, leave. The daemon notices and moves on.
#[tokio::test]
async fn a_silent_client_costs_nothing() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, None).await;
    {
        let _silent = UnixStream::connect(&d.path).await.unwrap();
    }
    // The next client is served normally, so the silent one took
    // nothing down with it.
    let env = ask(&d.path, r#"{"verb":"health","args":{}}"#).await;
    assert_eq!(env["success"], true, "{env}");
}

/// EC-4: two clients, two sessions, two connections, both answered.
#[tokio::test]
async fn two_clients_are_two_sessions() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, None).await;
    let (a, b) = tokio::join!(
        ask(&d.path, r#"{"verb":"status","args":{}}"#),
        ask(&d.path, r#"{"verb":"health","args":{}}"#)
    );
    assert_eq!(a["command"], "status");
    assert_eq!(b["command"], "health");
    assert_eq!(a["success"], true);
    assert_eq!(b["success"], true);
}

/// The daemon's graph scoping comes from its settings, not from a
/// frame, which is what keeps the request grammar the contract's.
#[tokio::test]
async fn the_session_graph_comes_from_the_daemon_s_settings() {
    let Some(store) = store_socket_dir() else {
        return;
    };
    let d = start(&store, Some("daemon_scoped")).await;
    let env = ask(&d.path, r#"{"verb":"status","args":{}}"#).await;
    assert_eq!(env["data"]["session_graph"], "daemon_scoped");
}

/// EC-5: a socket directory that is not there is a refusal to start,
/// not a daemon serving nothing.
#[tokio::test]
async fn a_missing_socket_directory_refuses_to_start() {
    let e = yeomna_daemon::server::bind(std::path::Path::new("/nonexistent/dir/yeomna.sock"))
        .expect_err("binding must fail");
    assert!(e.to_string().contains("/nonexistent/dir"), "{e}");
}

/// FR7: a stale socket from a crash is replaced, because the
/// alternative is an appliance that will not start after a power cut.
#[tokio::test]
async fn a_stale_socket_is_replaced() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("yeomna.sock");
    std::fs::write(&path, b"a corpse from a crash").unwrap();
    let listener = yeomna_daemon::server::bind(&path).expect("replaces the stale file");
    drop(listener);
    // And the mode is what the appliance model wants.
    let listener = yeomna_daemon::server::bind(&path).expect("binds again");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the socket is the operator's alone");
    drop(listener);
}
