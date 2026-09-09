# Specification: 017 `yeomna call` and the Config File

Owner: epic #37 (the client surface), H10 (the config layer). Ruled by
R21 D4 (config is TOML at `/etc/yeomna/yeomna.toml`, dev override
`YEOMNA_CONFIG`), R3 (the appliance ships a config file), and V3 (the
actor is the kernel's answer). The third PR in R21's order.
Status: draft, 2026-09-09.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

The verb layer has executed since Phase 2 and nothing outside a test
can call it. This spec gives it one surface: `yeomna call`, JSON in and
one envelope out, which is the whole closed contract exposed through a
single command that never changes as verbs land.

Two things make this the right shape. An agent is JSON-native and wants
the envelope, not a rendering of it, so the human-ergonomic per-verb
tree (epic #37, Phase 7) is a separate later concern. And the first
caller is Claude Code itself (D9), running on this machine, which needs
no socket protocol to reach a library: **embedded mode links
`yeomna-verbs` and opens its own session**, and the framed daemon
transport lands with Phase 5 behind the same command.

H10 lands here because this is its first real consumer, which is the
condition the ledger set.

## Task Scope

- A new crate `yeomna-cli` with binary `yeomna`, a workspace member,
  outside the no-SQL lint's allowlist (it emits none).
- `yeomna call <json>` and `yeomna call -` reading stdin: parse into
  the closed `Verb` enum, run it, print the envelope as JSON to stdout.
- `yeomna verbs`: print the 42 wire names, one per line, so a caller
  can discover the contract without reading the source.
- The config file: TOML, `/etc/yeomna/yeomna.toml`, `YEOMNA_CONFIG`
  overriding the path, with a documented default for every key so a
  machine with no file works. Keys: `socket_dir`, `port`, `database`,
  and an optional default `graph`.
- The actor from the kernel: the real uid read from `/proc/self/status`,
  resolved to a name through `/etc/passwd`. Never from the environment,
  because `USER` is the caller's to set and V3 says the actor is not.
- `--graph` to scope a session, overriding the config's default.

## Out of Scope

- The daemon and the framed transport (Phase 5, its own spec). This
  spec's mode is embedded, and the command's surface is chosen so the
  transport can change underneath it.
- The per-verb subcommand tree, table rendering, and the census
  inversion (Phase 7).
- H8's `tools` commands.
- Any verb behavior. This spec adds a caller, not a capability.

## Files to Modify

- `Cargo.toml`: the new member.
- `crates/yeomna-cli/` (new): `Cargo.toml`, `src/main.rs`,
  `src/config.rs`, `src/actor.rs`.
- `crates/yeomna-cli/tests/`: the config resolution tests, the actor
  test, and a cluster-gated end-to-end run of the binary.
- `docs/holes.md`: H10 filled, epic #37's first part done.

## Files to Reference

- `crates/yeomna-verbs/src/verb.rs`: the closed enum and its serde
  representation, which is the command's whole input grammar.
- `crates/yeomna-verbs/src/execute.rs`: `Session::new`, `with_graph`,
  `with_endpoint`.
- `crates/yeomna-embed/tests/config_and_types.rs`: the pinned client
  defaults H10 must reproduce when it grows to cover them.

## Functional Requirements

- **FR1** `yeomna call '{"verb":"status","args":{}}'` prints the
  success envelope as JSON and exits 0. The same JSON on stdin with
  `-` does the same thing.
- **FR2** A verb that fails prints the failure envelope, error and all,
  and exits 1. The envelope is the answer either way, so a caller
  parses one shape.
- **FR3** Unparseable JSON, an unknown verb name, or a request with an
  unknown field is a usage error: a message on stderr naming what was
  wrong, exit 2, and no call made. The closed contract's
  `deny_unknown_fields` does this work, and the message carries serde's
  reason.
- **FR4** The config resolves in one order: `YEOMNA_CONFIG` if set,
  else `/etc/yeomna/yeomna.toml` if it exists, else the built-in
  defaults. A file that exists and does not parse is a usage error
  naming the file, never a silent fallback.
- **FR5** The actor is the real uid resolved to a name, and
  `USER`, `LOGNAME`, or any other environment variable cannot change
  it. An unresolvable uid is recorded as `uid:<n>` in the actor string
  rather than refused, because this is an in-process caller that the
  kernel already authenticated by filesystem permission on the socket.
  (The daemon's rule is stricter and different: D6 refuses a peer whose
  uid does not resolve, because there the uid is a stranger's.)
- **FR6** `yeomna verbs` prints all 42 wire names and exits 0, and a
  test proves the list matches the contract rather than a copy.
- **FR7** Every call is audited exactly as a test's would be, with the
  actor the kernel gave, which is G1 holding through a new caller.

## Edge Cases

- **EC-1** No cluster: the connection error is a clean message on
  stderr and exit 2, not a panic or a backtrace.
- **EC-2** A verb needing a session graph with none configured: the
  verb's own `InvalidArgs` envelope, exit 1. The CLI does not
  second-guess the contract.
- **EC-3** An empty or whitespace-only argument: usage error, exit 2.
- **EC-4** A config file naming a socket directory that does not
  exist: the connection fails as EC-1 does, naming the path tried.
- **EC-5** stdin closed with no data under `-`: usage error, exit 2.
- **EC-6** A verb that this phase refuses by name (`ingest`,
  `graph.materialize`): the `Unimplemented` envelope, exit 1. The
  refusal reaches the caller as an answer, which is the point of the
  taxonomy.

## Implementation Notes

DO:

- DO keep the binary thin. It parses, connects, calls, prints. Every
  decision about what a verb means stays in `yeomna-verbs`.
- DO print the envelope and nothing else on stdout, so a caller can
  pipe it into a parser without filtering.
- DO read the uid from `/proc/self/status`, which is the kernel's
  answer, and treat `/etc/passwd` as the naming table it is.
- DO run the gate three times before calling it done.

DON'T:

- DON'T add a second way to say what a verb is. The enum is the
  grammar.
- DON'T let a config key change verb behavior. Config says where the
  store is, not what the verbs do.
- DON'T read the actor from the environment under any circumstance.
- DON'T emit SQL. This crate is not on the lint's allowlist and must
  stay off it.

## Success Criteria

1. `yeomna call` reaches every implemented verb and refuses every
   unimplemented one with its named reason.
2. The config resolves per FR4 with the defaults documented in the
   crate and reproduced in a test.
3. H10 is filled: the appliance has a config file with one consumer.
4. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR7 and EC-1 through EC-6, cluster-gated
  where they touch the store, per-cause skip messages.
- The no-SQL lint passes with `yeomna-cli` outside its allowlist.
