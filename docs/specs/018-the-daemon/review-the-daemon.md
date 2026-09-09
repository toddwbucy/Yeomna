# Review notes: 018 the daemon

Status: build complete 2026-09-09, local review run before the PR
opened. **Stacked on spec 017** and merges after it. Rulings applied:
R21 D5 (socket, frames, unit), D6 (an unresolvable peer uid is
refused), D1 (long-running verbs block), V3 (the actor is the kernel's
answer).

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## What landed

- `yeomna-daemon`, a workspace member with binary `yeomnad` and a
  library so its own tests can start a listener in process. Outside the
  no-SQL lint's allowlist and emitting none.
- The frame codec, in `yeomna-verbs` rather than here (below).
- The listener: socket at 0600, a stale socket from a crash replaced
  rather than refused, a missing directory refused rather than served.
- Peercred at accept, resolved through the same `actor` table the CLI
  uses. D6 holds.
- One `Session` per connection.
- `yeomna call --daemon`, the same JSON over the socket.
- `deploy/yeomna-daemon.service`, mirroring the Postgres unit.

## Build findings

**1. The frame codec belongs to the protocol crate.** A first cut put
it in `yeomna-daemon`, which would have made the CLI depend on the
daemon to speak to it. The frame format is the protocol, and
`yeomna-verbs` already owns the protocol's types, so `frame` lives
there and both ends use it without either depending on the other.

**2. `status` now reports the actor, and the lint is why.** The
end-to-end proof that a peer's uid names its audit rows needed to read
`audit_log`, which this crate may not do. That is the second time in
two specs the same wall appeared, and the second time the answer was
the same: **no verb reads the audit log**, so there was no way for a
caller to see who the appliance thinks it is. `status` reports `actor`
now, beside the role it already reported. An operator asking the
appliance who is calling gets an answer, and both transports prove V3
through the contract rather than around it.

**3. A test that deadlocked, and what it was really saying.** The
transport test starts a daemon in process and then runs the CLI as a
child. Waiting on that child from a worker thread occupies the runtime
the daemon task needs, and the two wait on each other. The child now
waits on the blocking pool. Worth recording because the shape recurs:
any test that blocks on a process while serving it from the same
runtime has this bug, and the symptom is a hang rather than a failure.

**4. The embedded and framed actor rules differ on purpose.** Embedded
mode records an unresolvable uid as `uid:<n>` because the caller was
already the process and there is no stranger to admit. The daemon
refuses it (D6) because the uid belongs to a peer. Both rules are in
spec 017 and spec 018 next to each other so the difference reads as a
decision.

## The dogfood

A daemon started by hand on a temporary socket, called through the CLI:

```
$ ls -l yeomna.sock
srw------- 1 todd todd 0 ... yeomna.sock
$ yeomna --daemon call '{"verb":"status","args":{}}'
{ "success": true, "command": "status",
  "data": { "actor": "todd", "role": "yeomna_app", ... } }
$ # the daemon's log
INFO yeomna_daemon::server: yeomnad is listening
INFO yeomna_daemon::server: accepted actor=todd uid=1000
```

The socket is 0600, the peer was named from uid 1000 by the kernel, and
the envelope carries the actor that every audit row for that connection
will carry.

## Test inventory

- `yeomna-verbs` unit (5, in `frame`): a round trip, many frames in
  sequence, an oversized claim refused without being honored (with the
  maximum itself admissible, so the bound is a limit rather than an
  off-by-one), a truncated frame, and a clean close.
- `yeomna-daemon/tests/daemon.rs` (9): FR1 through FR7 and EC-1, EC-4,
  EC-5 against a real socket.
- `yeomna-cli/tests/cli.rs` (2 new): FR8 comparing both transports
  answer for answer, and an absent daemon naming the socket it tried.

## Riding items

- EC-3 (a verb that takes minutes) is D1's blocking rule and has no
  test, because nothing implemented takes minutes yet. Phase 6's
  `ingest` is the first verb that will, and it is the right place to
  prove the connection waits rather than times out.
- No verb reads the audit log. Recorded in spec 017's notes and again
  here, since it has now shaped two builds. A contract question for
  Todd rather than a fix.
- The unit names `/usr/local/bin/yeomnad` and the operator's home for
  its socket directory. A deployment gives the daemon a system user,
  which is also the durable fix for the RemoveIPC incident, and the
  unit says so in its comments.
