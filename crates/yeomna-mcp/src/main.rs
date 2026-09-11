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

use yeomna_mcp::{RemoteConfig, Target, serve};

const USAGE: &str = r"yeomna-mcp serves the verb contract as MCP tools over stdio.

USAGE
    yeomna-mcp --local
    yeomna-mcp --ssh <destination>

    --local                  run `yeomna call -` on this machine
    --ssh <destination>      run it through ssh, so the call lands there
                             as a real uid and the audit row is named by
                             that machine's kernel
    --config <path>          the appliance config the call should read, as
                             a path on the machine that runs it

Exactly one target is required. Connection reuse for --ssh is ssh's own
ControlMaster, configured in your ~/.ssh/config.

--config is how a target is pointed at a database and a session graph.
With --ssh it is the only way: ssh forwards no environment, so a
YEOMNA_CONFIG set beside this process names a path on the wrong machine
and is read by nobody. Without it the remote falls back to its own
resolution, which may be a different database and no session graph, and a
hybrid query then refuses for a reason two machines from the symptom.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (target, config) = match parse(&args) {
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
    match runtime.block_on(serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        target,
        config,
    )) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("the session ended badly: {e}");
            ExitCode::from(1)
        }
    }
}

/// Exactly one of `--local` and `--ssh` (FR12), plus an optional config.
fn parse(args: &[String]) -> Result<(Target, RemoteConfig), String> {
    let mut target: Option<Target> = None;
    let mut config: Option<String> = None;
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
            "--config" => {
                if config.is_some() {
                    return Err("name one config, not two".into());
                }
                let Some(path) = args.get(i + 1) else {
                    return Err("--config needs a path".into());
                };
                if path.starts_with('-') {
                    return Err(format!("--config needs a path, and {path:?} is a flag"));
                }
                config = Some(path.clone());
                i += 1;
            }
            "-h" | "--help" => return Err("".into()),
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    let target =
        target.ok_or_else(|| "name a target: --local or --ssh <destination>".to_string())?;
    Ok((target, RemoteConfig::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exactly_one_target_is_required() {
        assert_eq!(parse(&args(&["--local"])).unwrap().0, Target::local());
        assert_eq!(
            parse(&args(&["--ssh", "olympus"])).unwrap().0,
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
    fn a_config_is_optional_and_its_shape_is_checked() {
        assert_eq!(
            parse(&args(&["--local"])).unwrap().1,
            RemoteConfig::default()
        );
        let (_, c) = parse(&args(&[
            "--ssh",
            "olympus",
            "--config",
            "/etc/yeomna/yeomna.toml",
        ]))
        .unwrap();
        assert_eq!(
            c,
            RemoteConfig::new(Some("/etc/yeomna/yeomna.toml".into())).unwrap()
        );
        assert!(
            parse(&args(&["--local", "--config"])).is_err(),
            "needs a path"
        );
        assert!(
            parse(&args(&["--local", "--config", "relative.toml"])).is_err(),
            "must be absolute"
        );
        assert!(
            parse(&args(&["--local", "--config", "/tmp/a;id"])).is_err(),
            "no shell syntax"
        );
    }

    #[test]
    fn an_unknown_argument_is_named() {
        let e = parse(&args(&["--mcp-everything"])).unwrap_err();
        assert!(e.contains("--mcp-everything"), "got {e:?}");
    }
}
