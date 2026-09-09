//! `yeomnad`, the appliance's socket server (spec 018, PRD Phase 5).
//!
//! Transport and nothing else: it frames, it names the caller from the
//! kernel, and it dispatches into `yeomna-verbs`. Every decision about
//! what a verb means lives there, which is the deliberate contrast with
//! the reference's 7.9k-line dispatch file.
//!
//! The socket is the only surface, and the unit that ships with this
//! binary carries `RestrictAddressFamilies=AF_UNIX`, so charter 5.2's
//! default-deny is a property of the service rather than a promise in a
//! configuration file.

use yeomna_daemon::server;

use std::path::PathBuf;
use std::process::ExitCode;

/// The config file, shared with the CLI (H10). Read here rather than in
/// `server`, so the listener takes settings and knows nothing about
/// where they came from.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default = "default_socket_dir")]
    socket_dir: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_database")]
    database: String,
    #[serde(default)]
    graph: Option<String>,
    /// Where this daemon listens. Beside the store's socket by default,
    /// which is a directory the appliance already keeps at 0700.
    #[serde(default)]
    socket_path: Option<String>,
}

fn default_socket_dir() -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => format!("{home}/.local/share/yeomna/run"),
        _ => "/run/yeomna".to_string(),
    }
}

fn default_port() -> u16 {
    5433
}

fn default_database() -> String {
    "yeomna".to_string()
}

fn load() -> Result<Config, String> {
    let named = std::env::var("YEOMNA_CONFIG")
        .ok()
        .filter(|p| !p.is_empty());
    let path = match named {
        Some(p) => PathBuf::from(p),
        None => {
            let shipped = PathBuf::from("/etc/yeomna/yeomna.toml");
            // Same rule the CLI's loader carries: `exists()` answers
            // false for a file that is there and unreadable, and falling
            // back would serve a store the operator did not name. Only a
            // genuine absence is a fallback.
            match std::fs::metadata(&shipped) {
                Ok(_) => shipped,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(toml::from_str("").expect("an empty config is all defaults"));
                }
                Err(e) => return Err(format!("cannot read {}: {e}", shipped.display())),
            }
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("YEOMNA_LOG").unwrap_or_else(|_| "yeomna_daemon=info".to_string()),
        )
        .init();

    let config = match load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("yeomnad: {e}");
            return ExitCode::from(2);
        }
    };
    let socket_path = PathBuf::from(
        config
            .socket_path
            .clone()
            .unwrap_or_else(|| format!("{}/yeomna.sock", config.socket_dir)),
    );

    let listener = match server::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("yeomnad: cannot listen on {}: {e}", socket_path.display());
            return ExitCode::from(2);
        }
    };
    let settings = server::Settings {
        socket_dir: config.socket_dir,
        port: config.port,
        database: config.database,
        graph: config.graph,
    };

    // FR7: a clean stop takes the socket with it, so the next start
    // finds the directory as it left it.
    tokio::select! {
        _ = server::serve(listener, settings) => {}
        _ = shutdown() => {}
    }
    let _ = std::fs::remove_file(&socket_path);
    ExitCode::from(0)
}

/// systemd sends TERM, a terminal sends INT, and both mean stop.
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut int = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(_) => return,
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
