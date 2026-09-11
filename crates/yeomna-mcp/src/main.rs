//! `yeomna-mcp`, the MCP front end (spec 024).
//!
//! An MCP client launches this as a subprocess and speaks
//! newline-delimited JSON-RPC over its standard streams. It binds no
//! listener and opens no port, which is why it needs no charter
//! amendment: the appliance's seal is untouched and the only thing
//! crossing a network is ssh, which the operator already runs.
//!
//! Usage:
//!   yeomna-mcp --local              run calls on this machine
//!   yeomna-mcp --ssh <destination>  run them through ssh
//!
//! Which database and which session graph a call reaches is the target's
//! business, from the config on the machine where `yeomna call` runs
//! (D13). This process passes neither and adds no override.

use std::process::ExitCode;

use yeomna_mcp::{Target, serve};

const USAGE: &str = r"yeomna-mcp serves the verb contract as MCP tools over stdio.

USAGE
    yeomna-mcp --local
    yeomna-mcp --ssh <destination>

    --local                  run `yeomna call -` on this machine
    --ssh <destination>      run it through ssh, so the call lands there
                             as a real uid and the audit row is named by
                             that machine's kernel

Exactly one target is required. Connection reuse for --ssh is ssh's own
ControlMaster, configured in your ~/.ssh/config.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let target = match parse(&args) {
        Ok(t) => t,
        Err(e) => {
            // Usage goes to stderr. Nothing but MCP messages may reach
            // stdout, even before a session starts.
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot start the runtime: {e}");
            return ExitCode::from(2);
        }
    };
    match runtime.block_on(serve(tokio::io::stdin(), tokio::io::stdout(), target)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("the session ended badly: {e}");
            ExitCode::from(1)
        }
    }
}

/// Exactly one of `--local` and `--ssh` (FR12).
fn parse(args: &[String]) -> Result<Target, String> {
    let mut target: Option<Target> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--local" => {
                if target.is_some() {
                    return Err("name one target, not two".into());
                }
                target = Some(Target::local());
            }
            "--ssh" => {
                if target.is_some() {
                    return Err("name one target, not two".into());
                }
                let Some(dest) = args.get(i + 1) else {
                    return Err("--ssh needs a destination".into());
                };
                // A flag is not a destination, and defaulting to a host
                // is how a call reaches a machine nobody named.
                if dest.starts_with('-') {
                    return Err(format!("--ssh needs a destination, and {dest:?} is a flag"));
                }
                target = Some(Target::ssh(dest));
                i += 1;
            }
            "-h" | "--help" => return Err("".into()),
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    target.ok_or_else(|| "name a target: --local or --ssh <destination>".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exactly_one_target_is_required() {
        assert_eq!(parse(&args(&["--local"])).unwrap(), Target::local());
        assert_eq!(
            parse(&args(&["--ssh", "olympus"])).unwrap(),
            Target::ssh("olympus")
        );
        assert!(parse(&args(&[])).is_err(), "neither");
        assert!(
            parse(&args(&["--local", "--ssh", "olympus"])).is_err(),
            "both"
        );
    }

    #[test]
    fn ssh_without_a_destination_is_a_usage_error() {
        assert!(parse(&args(&["--ssh"])).is_err());
        // A flag where a destination belongs would otherwise be taken as
        // a hostname, which is the mistake the CLI tree already made once.
        assert!(parse(&args(&["--ssh", "--local"])).is_err());
    }

    #[test]
    fn an_unknown_argument_is_named() {
        let e = parse(&args(&["--mcp-everything"])).unwrap_err();
        assert!(e.contains("--mcp-everything"), "got {e:?}");
    }
}
