//! The appliance's config file (H10, R3, R21 D4).
//!
//! One TOML file at `/etc/yeomna/yeomna.toml`, `YEOMNA_CONFIG` naming
//! another path, and a documented default for every key so a machine
//! with no file still works. The file says where the store is. It never
//! says what a verb does, because that is the contract's business and a
//! config key that changed verb behavior would be a second surface.

use std::path::PathBuf;

/// Where the appliance ships its config.
pub const DEFAULT_PATH: &str = "/etc/yeomna/yeomna.toml";

/// Where the store answers, and which graph a session starts scoped to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The directory holding the Postgres socket. Defaults to the dev
    /// cluster's, because that is the one that exists before a
    /// deployment writes a file.
    #[serde(default = "default_socket_dir")]
    pub socket_dir: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_database")]
    pub database: String,
    /// The graph a session starts scoped to, when the caller names
    /// none. Absent means unscoped, which several verbs refuse and
    /// several do not.
    #[serde(default)]
    pub graph: Option<String>,
    /// Where the daemon listens (spec 018). Beside the store's socket
    /// by default, which is a directory the appliance already keeps at
    /// 0700. Only `--daemon` reads it.
    #[serde(default)]
    pub socket_path: Option<String>,
}

impl Config {
    /// The daemon's socket, named or derived.
    pub fn daemon_socket(&self) -> String {
        self.socket_path
            .clone()
            .unwrap_or_else(|| format!("{}/yeomna.sock", self.socket_dir))
    }
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

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_dir: default_socket_dir(),
            port: default_port(),
            database: default_database(),
            graph: None,
            socket_path: None,
        }
    }
}

/// Why a config did not load. Every variant names the path it tried,
/// because a config that silently fell back to defaults is the failure
/// mode this type exists to prevent.
#[derive(Debug)]
pub enum ConfigError {
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    Malformed {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Unreadable { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
            ConfigError::Malformed { path, reason } => {
                write!(f, "cannot parse {}: {reason}", path.display())
            }
        }
    }
}

/// The resolution order, and the only one: `YEOMNA_CONFIG` if set, then
/// the shipped path if it exists, then the built-in defaults. A file
/// that exists and will not parse is an error rather than a fallback,
/// since falling back would run the appliance against a store the
/// operator did not name.
pub fn load() -> Result<Config, ConfigError> {
    load_from(
        std::env::var("YEOMNA_CONFIG")
            .ok()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
    )
}

/// The same resolution with the named path passed in, so a test can
/// exercise it without mutating the environment of a binary whose tests
/// run in parallel.
pub fn load_from(named: Option<PathBuf>) -> Result<Config, ConfigError> {
    let path = match named {
        Some(p) => p,
        None => {
            let shipped = PathBuf::from(DEFAULT_PATH);
            // `exists()` answers false for a file that is there and
            // unreadable, which would fall back to the defaults and run
            // the appliance against a store the operator did not name.
            // Only a genuine absence is a fallback.
            match std::fs::metadata(&shipped) {
                Ok(_) => shipped,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Config::default());
                }
                Err(source) => {
                    return Err(ConfigError::Unreadable {
                        path: shipped,
                        source,
                    });
                }
            }
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Unreadable {
        path: path.clone(),
        source,
    })?;
    toml::from_str(&text).map_err(|e| ConfigError::Malformed {
        path,
        reason: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped defaults, pinned. A deployment's file has to
    /// reproduce whatever it wants to keep, so what they are is part of
    /// the contract rather than an implementation detail.
    #[test]
    fn the_defaults_are_what_they_say() {
        let c = Config::default();
        assert!(
            c.socket_dir.ends_with("/.local/share/yeomna/run") || c.socket_dir == "/run/yeomna"
        );
        assert_eq!(c.port, 5433);
        assert_eq!(c.database, "yeomna");
        assert_eq!(c.graph, None);
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_what_it_omits() {
        let c: Config = toml::from_str("graph = \"yeomna_self\"\n").unwrap();
        assert_eq!(c.graph.as_deref(), Some("yeomna_self"));
        assert_eq!(c.port, 5433, "omitted keys keep their default");
        assert_eq!(c.database, "yeomna");
    }

    #[test]
    fn a_full_file_overrides_every_key() {
        let c: Config = toml::from_str(
            "socket_dir = \"/run/yeomna\"\nport = 6000\ndatabase = \"other\"\ngraph = \"g\"\n",
        )
        .unwrap();
        assert_eq!(
            c,
            Config {
                socket_dir: "/run/yeomna".into(),
                port: 6000,
                database: "other".into(),
                graph: Some("g".into()),
                socket_path: None,
            }
        );
    }

    /// An unreadable named file is an error, not a fallback. The
    /// shipped path's own case is the same code and cannot be tested
    /// without writing to /etc, so this covers the branch that decides.
    #[test]
    fn an_unreadable_file_is_refused_rather_than_fallen_back_from() {
        let d = tempfile::TempDir::new().unwrap();
        let path = d.path().join("locked.toml");
        std::fs::write(&path, "port = 5433\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything, so a machine running tests as root would
        // see this succeed and prove nothing.
        if std::fs::read_to_string(&path).is_ok() {
            eprintln!("SKIP: this user can read a 0000 file");
            return;
        }
        let e = load_from(Some(path)).expect_err("an unreadable file is an error");
        assert!(
            matches!(e, ConfigError::Unreadable { .. }),
            "got {e}, which reads as a parse failure rather than an access one"
        );
    }

    /// A misspelled key is a mistake to report, not a line to ignore,
    /// the same discipline the verb contract's requests carry.
    #[test]
    fn an_unknown_key_is_refused() {
        let e = toml::from_str::<Config>("sockets_dir = \"/tmp\"\n").unwrap_err();
        assert!(e.to_string().contains("sockets_dir"), "{e}");
    }
}
