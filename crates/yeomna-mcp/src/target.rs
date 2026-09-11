//! Where a request is executed, and how its exit status is read
//! (spec 024 D3, D12, D7).
//!
//! This crate never executes a verb. It spawns `yeomna call -`, writes
//! one request to that process's stdin, and closes it. Both targets use
//! the same fixed command, so there is one execution path shared with
//! every other surface rather than a fourth place a verb can run.
//!
//! **The request never travels in argv.** ssh joins its command
//! arguments into one string and hands it to a shell on the far side, so
//! a request carrying a quote, a backtick, a dollar sign, or a semicolon
//! would be interpreted there rather than delivered. Request text is
//! corpus-derived and caller-supplied, which makes argv an injection path
//! rather than a formatting choice.

use std::process::Stdio;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

/// The CLI's own exit codes, which D7 maps onto MCP's two error channels.
const ANSWERED: i32 = 0;
const REFUSED: i32 = 1;
const CALLER_OR_MACHINE: i32 = 2;
/// How much of a child's error output is kept.
///
/// The pipe is always drained to the end, because a child that fills it
/// blocks and never exits. What is bounded is what is *retained*: only
/// the first line ever reaches a message, and an ingest that logs can
/// produce megabytes. Keeping it all would buffer that for nothing and,
/// worse, the empty-stdout branch below used to put the whole of it into
/// a JSON-RPC error, which lands in the model's context.
const STDERR_KEPT: usize = 8 * 1024;

/// The largest answer this layer will carry.
///
/// Matched to the daemon's frame cap (R21 D5), so the two ways into the
/// appliance agree about how large an answer can be rather than each
/// picking a number. **Over the cap is a refusal, never a truncation**,
/// on the embedder PRD's D5 reasoning: a truncated envelope is not a
/// smaller answer, it is a malformed one, and handing a model half a
/// JSON document with nothing saying so is worse than an error.
const STDOUT_LIMIT: usize = 16 * 1024 * 1024;

/// ssh's own failure. `yeomna call` returns only 0, 1, or 2, so 255 is
/// distinguishable in practice, and the warrant is what the CLI chooses
/// to return rather than anything ssh reserves.
const SSH_FAILED: i32 = 255;

/// A config path this appliance will carry to the machine that runs the
/// call, and the characters it may contain.
///
/// **This exists because a remote target could not be pointed at a
/// graph.** `--local` inherits `YEOMNA_CONFIG` from its own environment,
/// but ssh forwards no environment: OpenSSH sends `LANG` and `LC_*` and
/// nothing else unless both ends are configured for it. So an env var set
/// beside a remote target names a path on the wrong machine and is read by
/// nobody, and the remote `yeomna` falls back to its own defaults, which
/// is a different database and no session graph. A hybrid query then
/// refuses and the reason is two machines away from the symptom.
///
/// The value is operator-supplied at launch rather than caller-supplied
/// per request, so it is not the injection class D12 is about. It still
/// reaches a shell on the far side, because ssh always runs its command
/// through one, so the shape is restricted to what a path needs and
/// nothing a shell reads as syntax.
fn config_path_is_plain(p: &str) -> bool {
    p.starts_with('/')
        && !p.is_empty()
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
}

/// Where `yeomna call` runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// On this machine.
    Local { program: String },
    /// Through ssh, so the call lands on the far side as a real uid and
    /// the audit row is named by the kernel there (D2).
    Ssh {
        ssh: String,
        destination: String,
        program: String,
    },
}

/// The config a target should read, as a path on the machine that runs the
/// call. `None` leaves that machine's own resolution alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteConfig(pub Option<String>);

impl RemoteConfig {
    /// Refuses anything a shell would read as more than a path.
    pub fn new(path: Option<String>) -> Result<Self, String> {
        match &path {
            Some(p) if !config_path_is_plain(p) => Err(format!(
                "a config path must be absolute and contain only letters, digits, and \
                 the characters . _ - /, and {p:?} is not"
            )),
            _ => Ok(Self(path)),
        }
    }
}

impl Target {
    pub fn local() -> Self {
        Self::Local {
            program: "yeomna".into(),
        }
    }

    pub fn ssh(destination: impl Into<String>) -> Self {
        Self::Ssh {
            ssh: "ssh".into(),
            destination: destination.into(),
            program: "yeomna".into(),
        }
    }

    /// The same target with another program name, used by tests to point
    /// at a stand-in.
    ///
    /// Not reachable from the command line, though that buys less than it
    /// looks: `Target::local` names the program without a path, so `PATH`
    /// decides which binary runs, and an MCP client launched from a
    /// desktop session often has a `PATH` that does not include it at
    /// all. Which binary answers is the operator's to fix by launching
    /// this process with an environment that resolves it.
    pub fn with_program(self, p: impl Into<String>) -> Self {
        match self {
            Self::Local { .. } => Self::Local { program: p.into() },
            Self::Ssh {
                ssh, destination, ..
            } => Self::Ssh {
                ssh,
                destination,
                program: p.into(),
            },
        }
    }

    /// The same target reached through another ssh binary. Tests use it
    /// to stand in for ssh itself, and like `with_program` it is not
    /// reachable from the command line.
    pub fn with_ssh_program(self, p: impl Into<String>) -> Self {
        match self {
            Self::Ssh {
                destination,
                program,
                ..
            } => Self::Ssh {
                ssh: p.into(),
                destination,
                program,
            },
            other => other,
        }
    }

    fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    /// `yeomna call -` locally, or through ssh. The trailing `-` is what
    /// makes the CLI read the request from stdin.
    fn command(&self, config: &RemoteConfig) -> Command {
        let mut c = match self {
            Self::Local { program } => {
                let mut c = Command::new(program);
                c.arg("call").arg("-");
                // Locally the child inherits this process's environment,
                // so naming the config is a plain env set.
                if let Some(path) = &config.0 {
                    c.env("YEOMNA_CONFIG", path);
                }
                c
            }
            Self::Ssh {
                ssh,
                destination,
                program,
            } => {
                let mut c = Command::new(ssh);
                c.arg(destination);
                // ssh forwards no environment, so the assignment has to
                // travel as part of the remote command.
                if let Some(path) = &config.0 {
                    c.arg("env").arg(format!("YEOMNA_CONFIG={path}"));
                }
                c.arg(program).arg("call").arg("-");
                c
            }
        };
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        c
    }
}

/// What one call produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The verb answered. Exit 0.
    Answered(Value),
    /// The verb refused or failed. Exit 1, and a result the model can
    /// correct against rather than a protocol error.
    Refused(Value),
    /// The caller or the machine was wrong and no call was made, or the
    /// transport failed, or the status meant nothing this build knows.
    /// All three are protocol errors because none is a verb outcome.
    Failed(String),
}

/// A spawned call, kept separate from its completion so a cancellation
/// can reach the child. Dropping it kills the child, because the command
/// is built with `kill_on_drop`.
pub struct Running {
    child: Child,
    is_ssh: bool,
    /// Written to the child's stdin while its output is being drained,
    /// never before.
    body: Vec<u8>,
}

impl Running {
    /// Kill and reap. Cancellation and stdin EOF both land here, and it
    /// terminates what this process owns (D10). A remote verb already
    /// inside its transaction may still run to completion, because ssh
    /// does not forward signals without a tty. The audit row is what
    /// makes that visible: it commits before the verb runs, so an
    /// abandoned call leaves a NULL outcome naming its actor.
    pub async fn terminate(&mut self) {
        let _ = self.child.kill().await;
    }

    /// Send the request, drain both pipes, and read the exit status.
    ///
    /// Borrows rather than consumes so a cancellation racing this in a
    /// `select!` can drop the whole future and take the child with it.
    ///
    /// **Writing and draining happen together, and that is the whole
    /// shape of this function.** Three pipes can each block the child,
    /// and doing any of them to completion before the others deadlocks:
    /// a child that fills its stderr buffer blocks, so it never exits, so
    /// stdout never reaches end of input. An ingest that logs is exactly
    /// that child. The same is true of the request going in: `embed.text`
    /// at the 16,384-token ceiling is well past a 64 KiB pipe, and for
    /// the ssh target the far side's output fills our unread pipes while
    /// ssh is still taking our stdin, so writing first can block forever.
    /// The first build wrote the request before returning `Running`,
    /// which put that write outside both the drain and the cancellation.
    pub async fn finish(&mut self) -> Outcome {
        use tokio::io::AsyncReadExt;
        let is_ssh = self.is_ssh;
        let mut stdin_pipe = self.child.stdin.take();
        let mut stdout_pipe = self.child.stdout.take();
        let mut stderr_pipe = self.child.stderr.take();
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let body = std::mem::take(&mut self.body);

        let send = async {
            if let Some(mut p) = stdin_pipe.take() {
                let r = p.write_all(&body).await;
                // The close is the end of the request: `yeomna call -`
                // reads until end of input, so a child whose stdin stays
                // open waits rather than answering.
                drop(p);
                r
            } else {
                Ok(())
            }
        };
        // stdout is kept whole up to the cap, because it carries the
        // envelope. Past the cap the bytes are dropped and `overflowed`
        // is set, so the call refuses rather than parsing a fragment. The
        // pipe keeps being read either way, because a child that fills it
        // blocks and never exits.
        let mut overflowed = false;
        let out = async {
            if let Some(p) = stdout_pipe.as_mut() {
                let mut chunk = [0u8; 8192];
                loop {
                    match p.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if stdout_buf.len() + n > STDOUT_LIMIT {
                                overflowed = true;
                            } else {
                                stdout_buf.extend_from_slice(&chunk[..n]);
                            }
                        }
                    }
                }
            }
        };
        // Drained to the end, retained up to a cap. Draining is what
        // keeps the child from blocking, and retaining is what would
        // otherwise grow without bound. stdout is kept whole, because it
        // carries the envelope the caller is owed.
        let err = async {
            if let Some(p) = stderr_pipe.as_mut() {
                let mut chunk = [0u8; 8192];
                loop {
                    match p.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let room = STDERR_KEPT.saturating_sub(stderr_buf.len());
                            if room > 0 {
                                stderr_buf.extend_from_slice(&chunk[..n.min(room)]);
                            }
                        }
                    }
                }
            }
        };
        let (sent, (), ()) = tokio::join!(send, out, err);
        if overflowed {
            return Outcome::Failed(format!(
                "the answer is larger than this surface carries, which is {} MiB. It is \
                 refused rather than truncated, because half an envelope is malformed rather \
                 than smaller. Narrow the request, with a limit or a kind",
                STDOUT_LIMIT / (1024 * 1024)
            ));
        }

        let status = match self.child.wait().await {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(format!("could not run the call: {e}")),
        };
        // **A write failure is not reported over the child's own answer.**
        // A child that refuses early answers and exits without reading
        // the rest of its input, which breaks the pipe under us. Reporting
        // that as "cannot send the request" would throw away a perfectly
        // good envelope and name the wrong thing. The send error is only
        // surfaced when the child produced nothing usable.
        let send_failed = sent.err();
        let stdout = String::from_utf8_lossy(&stdout_buf);
        let stderr = String::from_utf8_lossy(&stderr_buf);
        let code = status.code();

        // Read the envelope only where an envelope is promised. A parse
        // failure quotes what arrived, truncated, because a failure that
        // hides the bytes is unfixable from the outside (EC-2).
        let envelope = |text: &str| -> Result<Value, String> {
            serde_json::from_str::<Value>(text.trim()).map_err(|e| {
                let seen: String = text.trim().chars().take(400).collect();
                format!("the target's answer is not an envelope: {e}. It said: {seen:?}")
            })
        };

        match code {
            Some(ANSWERED) => match envelope(&stdout) {
                Ok(v) => Outcome::Answered(v),
                Err(e) => Outcome::Failed(with_send_note(e, send_failed)),
            },
            Some(REFUSED) => match envelope(&stdout) {
                Ok(v) => Outcome::Refused(v),
                // A refusal that produced no envelope is not a refusal
                // this layer can hand a model. EC-1: never an empty
                // success.
                Err(_) if stdout.trim().is_empty() => Outcome::Failed(with_send_note(
                    format!(
                        "the call failed and said nothing on stdout. Its error output began: {}",
                        first_line(&stderr, "")
                    ),
                    send_failed,
                )),
                Err(e) => Outcome::Failed(with_send_note(e, send_failed)),
            },
            Some(CALLER_OR_MACHINE) => Outcome::Failed(with_send_note(
                format!(
                    "the call was refused before it ran: {}",
                    first_line(&stderr, &stdout)
                ),
                send_failed,
            )),
            // EC-3. Named as the transport, and naming the destination,
            // so a caller does not debug the wrong machine.
            Some(SSH_FAILED) if is_ssh => Outcome::Failed(format!(
                "ssh could not reach the appliance, so no call was made: {}",
                first_line(&stderr, &stdout)
            )),
            Some(other) => Outcome::Failed(format!(
                "the call exited {other}, which is not a status this contract defines: {}",
                first_line(&stderr, &stdout)
            )),
            None => Outcome::Failed(format!(
                "the call was killed by a signal: {}",
                first_line(&stderr, &stdout)
            )),
        }
    }
}

/// Append what went wrong sending the request, when something did and the
/// child's own answer did not explain the failure on its own.
fn with_send_note(why: String, send_failed: Option<std::io::Error>) -> String {
    match send_failed {
        Some(e) => format!("{why} (the request also failed to send: {e})"),
        None => why,
    }
}

/// The first non-empty line of either stream, for an error message that
/// says something without pasting a whole log into the model's context.
fn first_line(stderr: &str, stdout: &str) -> String {
    for s in [stderr, stdout] {
        if let Some(l) = s.lines().map(str::trim).find(|l| !l.is_empty()) {
            return l.chars().take(400).collect();
        }
    }
    "it said nothing".into()
}

/// Spawn the call, carrying the request for `finish` to send.
///
/// Nothing is written here: the write belongs beside the drain, for the
/// reasons on `finish`.
pub async fn start(
    target: &Target,
    config: &RemoteConfig,
    request: &Value,
) -> Result<Running, String> {
    let body = serde_json::to_vec(request).map_err(|e| format!("cannot serialize: {e}"))?;
    let child = target
        .command(config)
        .spawn()
        .map_err(|e| format!("cannot start the call: {e}"))?;
    Ok(Running {
        child,
        is_ssh: target.is_ssh(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A child that exits 1, writes nothing to stdout, and floods stderr.
    fn noisy(dir: &std::path::Path) -> Target {
        let p = dir.join("noisy");
        std::fs::write(
            &p,
            "#!/bin/sh\ncat > /dev/null\ni=0\nwhile [ $i -lt 4000 ]; do \
             echo 'a long line of error output that adds up quickly' >&2; i=$((i+1)); done\n\
             exit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        Target::local().with_program(p.display().to_string())
    }

    #[test]
    fn a_config_path_must_look_like_a_path() {
        assert!(RemoteConfig::new(None).is_ok(), "absent is fine");
        assert!(RemoteConfig::new(Some("/etc/yeomna/yeomna.toml".into())).is_ok());
        assert!(RemoteConfig::new(Some("/home/todd/.config/yeomna/a-b_1.toml".into())).is_ok());
        for bad in [
            "relative/path.toml",
            "/tmp/a;rm -rf /",
            "/tmp/$(id).toml",
            "/tmp/a`whoami`",
            "/tmp/a b.toml",
            "/tmp/a\"b",
        ] {
            assert!(
                RemoteConfig::new(Some(bad.into())).is_err(),
                "{bad} should be refused"
            );
        }
    }

    #[tokio::test]
    async fn a_failure_message_stays_small_however_much_the_child_wrote() {
        // The empty-stdout branch used to put the whole of stderr into a
        // JSON-RPC error, and that error lands in the model's context.
        // This child writes roughly 190 KB of it.
        let d = tempfile::tempdir().unwrap();
        let mut running = start(
            &noisy(d.path()),
            &RemoteConfig::default(),
            &serde_json::json!({"verb": "status", "args": {}}),
        )
        .await
        .unwrap();
        let out = running.finish().await;
        let Outcome::Failed(why) = out else {
            panic!("a child that exits 1 with no stdout is a failure, got {out:?}");
        };
        assert!(
            why.len() < 2_000,
            "the message carried {} bytes of the child's noise",
            why.len()
        );
        assert!(why.contains("said nothing on stdout"), "got {why:?}");
    }
}
