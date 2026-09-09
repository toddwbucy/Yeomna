# Specification: 018 The Daemon

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 5. Owner: H6 (the
daemon and transports), epic #37's second part. Ruled by R21 D5 (the
socket, the frames, the unit), D6 (an unresolvable peer uid is refused),
D1 (long-running verbs block), and V3 (the actor is the kernel's
answer). The fourth PR in R21's order.
Status: draft, 2026-09-09. **Stacked on spec 017**, which wrote the
config file and moved actor derivation into the verb layer, and merges
after it.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

`yeomna call` reaches the verb layer by linking it, which serves a
caller on this machine and nothing else. The daemon is the shape that
serves anything else: a Unix socket carrying length-prefixed JSON
frames, one verb request in and one envelope out, with the calling uid
read from the kernel at accept and written into every audit row for
that connection.

Two properties make this transport rather than architecture. The socket
is the trust boundary and the kernel enforces it, so nothing in a frame
says who is calling and there is nothing to spoof (V3). And the daemon
holds no business logic: it frames, it accepts, it dispatches into
`yeomna-verbs`, in deliberate contrast to the reference's 7.9k-line
dispatch file.

## Task Scope

- A new crate `yeomna-daemon` with binary `yeomnad`, a workspace
  member, outside the no-SQL lint's allowlist.
- The frame codec: a four-byte big-endian length followed by that many
  bytes of JSON, both directions, with a maximum frame of 16 MB
  (R21 D5). A frame that claims more is refused and the connection
  closes, because a length prefix is the one field a caller controls
  before any parsing happens.
- The listener: a socket at the configured path, mode 0600, created
  fresh at start and removed at a clean stop. A stale socket from a
  crash is replaced rather than refused, since the alternative is an
  appliance that will not start after a power cut.
- Peercred at accept: `SO_PEERCRED` through `UnixStream::peer_cred`,
  the uid resolved through the same `yeomna_verbs::actor` table the CLI
  uses. **D6: a uid that does not resolve is refused**, the connection
  closed, the refusal logged with the raw uid. The embedded caller's
  looser rule does not apply here, because there the caller was already
  the process and here it is a stranger.
- One `Session` per connection, so the session's serialization and its
  retirement rules hold per client, and the actor is fixed for the
  connection's life.
- Config: the socket path joins `yeomna-cli`'s config file (H10) as
  `socket_path`, with the same resolution order.
- The CLI gains `--daemon`, sending the request as a frame instead of
  linking the verb layer, so one command has two transports and the
  caller's JSON does not change.
- A systemd unit mirroring `yeomna-postgres`:
  `RestrictAddressFamilies=AF_UNIX`, `Restart=no`, and the run-as-user
  lesson from the 2026-09-09 RemoveIPC incident recorded in its
  comments.

## Out of Scope

- Any network transport. Charter 5.2 puts anything needing the wire on
  the far side of this socket, and the unit's
  `RestrictAddressFamilies` is what makes that a property rather than a
  promise.
- Authentication beyond peercred. Filesystem permission on the socket
  is the gate, which is the appliance model.
- Concurrency beyond one session per connection, and any job model.
  D1 ruled that long-running verbs block.
- The per-verb CLI tree (Phase 7) and H8's tools commands.

## Files to Modify

- `Cargo.toml`: the new member.
- `crates/yeomna-daemon/` (new): `Cargo.toml`, `src/main.rs`,
  `src/frame.rs`, `src/server.rs`.
- `crates/yeomna-cli/src/main.rs` and `src/config.rs`: `--daemon` and
  `socket_path`.
- `deploy/yeomna-daemon.service` (new): the unit.
- `docs/holes.md`: H6 filled, epic #37's second part done.

## Files to Reference

- `crates/yeomna-verbs/src/execute.rs`: `Session`, and why one per
  connection is the right grain.
- `crates/yeomna-verbs/src/actor.rs`: the uid-to-name table, shared
  with the CLI, which is why spec 017 moved it here.
- `CLAUDE.md`, the cluster section: the unit that this one mirrors,
  including what the RemoveIPC incident taught about running as a
  regular user.

## Functional Requirements

- **FR1** The daemon listens on the configured path, mode 0600, and a
  client that sends a framed request receives one framed envelope.
- **FR2** The actor on every audit row is the peer's uid resolved to a
  name, and no field in any frame can change it.
- **FR3** D6: a peer whose uid does not resolve to a name is refused,
  the connection closed without a call, and the refusal logged with the
  raw uid.
- **FR4** A frame claiming more than the maximum is refused before any
  allocation of that size, and the connection closes.
- **FR5** Malformed JSON inside a well-formed frame is answered with a
  failure envelope rather than a closed connection, because the caller
  framed correctly and deserves an answer in the shape it expects.
- **FR6** One connection carries many requests in sequence, and the
  session's serialization holds across them.
- **FR7** A clean stop removes the socket. A start over a stale socket
  replaces it.
- **FR8** `yeomna call --daemon` sends the same JSON the embedded mode
  takes and prints the same envelope, so the transport is a deployment
  choice rather than a contract change.

## Edge Cases

- **EC-1** A client that connects and disconnects without sending: no
  call, no audit row, no log noise beyond a debug line.
- **EC-2** A client that sends half a frame and vanishes: the read ends
  short, the connection closes, nothing partial reaches a verb.
- **EC-3** A verb that takes minutes (D1 blocks): the connection stays
  open and the client waits. Nothing times out on this side, since a
  timeout here would abandon a call the audit row says is running.
- **EC-4** Two clients at once: two connections, two sessions, two
  actors, and the store's own concurrency is what orders them.
- **EC-5** The socket directory does not exist or is not writable: the
  daemon refuses to start with a message naming the path, rather than
  starting and serving nothing.
- **EC-6** The store is unreachable at connection time: the client's
  frame is answered with an internal-error envelope, and the daemon
  stays up, because a store that comes back should find its daemon
  waiting.

## Implementation Notes

DO:

- DO read the length prefix before allocating, and refuse an
  oversized claim without honoring it.
- DO give each connection its own `Session`, so a retired session
  retires one client rather than the appliance.
- DO log a connection's actor once at accept rather than per request.
- DO run the gate three times before calling it done.

DON'T:

- DON'T put a verb decision in this crate. It frames and dispatches.
- DON'T read an actor from a frame under any circumstance.
- DON'T bind anything but `AF_UNIX`, and do not make that a runtime
  choice.
- DON'T emit SQL.

## Success Criteria

1. A framed client reaches every implemented verb and gets the same
   envelope the embedded caller gets.
2. The audit row carries the peer's actor, proven end to end.
3. An unresolvable uid is refused (D6), proven by test.
4. H6 is filled and the unit ships.
5. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR8 and EC-1 through EC-6, cluster-gated
  where they touch the store, per-cause skip messages.
- The no-SQL lint passes with `yeomna-daemon` outside its allowlist.
