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
/// ssh's own failure. `yeomna call` returns only 0, 1, or 2, so 255 is
/// distinguishable in practice, and the warrant is what the CLI chooses
/// to return rather than anything ssh reserves.
const SSH_FAILED: i32 = 255;

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

    /// The same target with another program name. Used by tests to point
    /// at a stand-in, and deliberately not reachable from the command
    /// line: what runs on the far side is not a caller's choice.
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
    fn command(&self) -> Command {
        let mut c = match self {
            Self::Local { program } => {
                let mut c = Command::new(program);
                c.arg("call").arg("-");
                c
            }
            Self::Ssh {
                ssh,
                destination,
                program,
            } => {
                let mut c = Command::new(ssh);
                c.arg(destination).arg(program).arg("call").arg("-");
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
/// can reach the child.
pub struct Running {
    child: Child,
    is_ssh: bool,
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

    /// Wait for the child and read its exit status as an outcome.
    ///
    /// Borrows rather than consumes so a cancellation racing this in a
    /// `select!` can still reach the child.
    pub async fn finish(&mut self) -> Outcome {
        let is_ssh = self.is_ssh;
        let mut stdout_pipe = self.child.stdout.take();
        let mut stderr_pipe = self.child.stderr.take();
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        // **Both pipes at once, and this is not a tidiness point.**
        // Draining stdout to end of file first would deadlock a child
        // that fills the stderr buffer: it blocks writing stderr, so it
        // never exits, so stdout never reaches end of file. An ingest
        // that logs is exactly that child.
        {
            use tokio::io::AsyncReadExt;
            let out = async {
                if let Some(p) = stdout_pipe.as_mut() {
                    let _ = p.read_to_end(&mut stdout_buf).await;
                }
            };
            let err = async {
                if let Some(p) = stderr_pipe.as_mut() {
                    let _ = p.read_to_end(&mut stderr_buf).await;
                }
            };
            tokio::join!(out, err);
        }
        let status = match self.child.wait().await {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(format!("could not run the call: {e}")),
        };
        let out = Out {
            status,
            stdout: stdout_buf,
            stderr: stderr_buf,
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let code = out.status.code();

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
                Err(e) => Outcome::Failed(e),
            },
            Some(REFUSED) => match envelope(&stdout) {
                Ok(v) => Outcome::Refused(v),
                // A refusal that produced no envelope is not a refusal
                // this layer can hand a model. EC-1: never an empty
                // success.
                Err(_) if stdout.trim().is_empty() => Outcome::Failed(format!(
                    "the call failed and said nothing on stdout. Its error output was: {:?}",
                    stderr.trim()
                )),
                Err(e) => Outcome::Failed(e),
            },
            Some(CALLER_OR_MACHINE) => Outcome::Failed(format!(
                "the call was refused before it ran: {}",
                first_line(&stderr, &stdout)
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

/// What a finished child produced.
struct Out {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
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

/// Spawn the call and hand it the request on stdin.
///
/// Exactly one request is written and stdin is then closed, because
/// `yeomna call -` reads until end of input and a child whose stdin stays
/// open waits rather than answering. **Omitting the close hangs the call
/// instead of failing it** (FR18).
pub async fn start(target: &Target, request: &Value) -> Result<Running, String> {
    let body = serde_json::to_vec(request).map_err(|e| format!("cannot serialize: {e}"))?;
    let mut child = target
        .command()
        .spawn()
        .map_err(|e| format!("cannot start the call: {e}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "the call has no stdin".to_string())?;
    if let Err(e) = stdin.write_all(&body).await {
        let _ = child.kill().await;
        return Err(format!("cannot send the request: {e}"));
    }
    // The close is the end of the request. Dropping the handle is what
    // sends it, and it is load bearing rather than tidiness.
    drop(stdin);
    Ok(Running {
        child,
        is_ssh: target.is_ssh(),
    })
}
