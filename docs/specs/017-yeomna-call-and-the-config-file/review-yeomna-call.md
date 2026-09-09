# Review notes: 017 `yeomna call` and the config file

Status: build complete 2026-09-09, local review run before the PR
opened per the workflow's step 3. Rulings applied: R3 and R21 D4 (the
config file), V3 (the actor is the kernel's answer), D9 (Claude Code is
the first caller).

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## What landed

- `yeomna-cli`, a workspace member with binary `yeomna`, outside the
  no-SQL lint's allowlist and emitting none.
- `yeomna call <json>` and `yeomna call -`, printing the envelope to
  stdout and exiting 0 for an answer, 1 for a refusal or failure, 2 for
  a caller or a machine that was wrong before any call was made.
- `yeomna verbs`, printing the contract.
- The config file, H10 filled, with `docs/yeomna.toml.example` as the
  shipped shape.
- `--graph`, overriding the config's default, and the config's default
  overriding nothing.

## Build findings, and what they changed

**1. The lint caught the test before the lint ran.** The end-to-end
proof of FR7 (a call is audited under the kernel's actor) needed to
read `audit_log`, and reading it is SQL, which this crate may not
carry. Gaming the pattern would have been dishonest and adding the
crate to the allowlist would have widened the charter's line for a
crate that must never need it. The resolution is a better shape:
**actor derivation moved into `yeomna-verbs` as `pub mod actor`**,
because V3 is a verb-layer contract and the daemon's peercred path
will want the same function at Phase 5. The audit assertion lives in
`yeomna-verbs/tests/audit.rs` where SQL is allowed, and the CLI suite
proves the half a caller can see: a hostile `USER` and `LOGNAME` do not
stop the call and do not name the caller.

**2. `WIRE_NAMES` moved from the contract test into the library.**
`yeomna verbs` cannot print a list the crate keeps privately without
that list becoming a second contract. The array is now
`yeomna_verbs::WIRE_NAMES`, the contract test reads it rather than
owning it, and the CLI test asserts the printed lines are that array.

**3. The audit test does not mutate the environment.** A first cut set
`USER` inside the test process to prove the derivation ignores it,
which is a process-wide change in a binary whose tests run in parallel.
The hostile-environment case belongs at process level, where the CLI
suite runs it against a child, and the verb-layer test asserts the
derivation and leaves the environment alone.

**4. The actor's unresolvable case differs from the daemon's, on
purpose.** FR5 records an unresolvable uid as `uid:<n>` rather than
refusing, because an embedded caller was already authenticated by
filesystem permission on the socket and there is no stranger to admit.
D6 rules the opposite for the daemon, where the uid belongs to a peer,
and the two rules are written next to each other in the spec so the
difference reads as a decision rather than an inconsistency.

## Test inventory

- `yeomna-cli` unit (4): the config's defaults pinned, a partial file
  keeping them, a full file overriding them, and an unknown key
  refused.
- `yeomna-verbs` unit (3, in `actor`): the passwd table naming a uid, a
  malformed table yielding no name rather than a wrong one, and the
  actor coming from the kernel.
- `yeomna-cli/tests/cli.rs` (9): the real binary, real arguments, real
  exit codes. FR1 through FR6, EC-1 through EC-6, including a smuggled
  `actor` field dying at the contract boundary and the store's absence
  naming the path it tried.
- `yeomna-verbs/tests/audit.rs` (1 new): the kernel's actor is what the
  row carries, which is FR7 and G1 holding through a new caller.

## The dogfood

Run against the live cluster before the PR opened:

```
yeomna --graph yeomna_self call '{"verb":"graph.neighbors","args":{
  "graph":"yeomna_self",
  "key":"crates_yeomna-verbs_src_execute_rs__Session__dispatch__d2656022",
  "direction":"in","relations":["calls"],"bases":[],"limit":5}}'
```

answers that `Session::call` calls `Session::dispatch`, which is the
graph describing its own code through the surface this spec added. That
is the loop D9 named: the first caller is a session building this
project, and from here it can ask.

## CodeRabbit round one

**`Path::exists()` answers false for a file that is there and
unreadable**, so an `/etc/yeomna/yeomna.toml` the process cannot read
would have fallen back to the defaults and run the appliance against a
store the operator did not name, which is the exact failure FR4 exists
to prevent. Only a genuine `NotFound` is a fallback now, and every
other access error is `ConfigError::Unreadable` naming the path. The
resolution split into `load` (which reads the environment) and
`load_from` (which does not), so the test proves it without mutating
the environment of a binary whose tests run in parallel, the discipline
this spec's own notes argued for. The test skips when the user can read
a 0000 file, since running as root would prove nothing.

## Riding items

- The daemon (spec 018) carries a second reader of this file format,
  and it inherited this same defect and the same fix. Two readers of one
  format is a duplication worth retiring when Phase 7's CLI tree makes
  it a third, and the natural shape is a small shared crate rather than
  either binary depending on the other.

- The framed daemon transport (Phase 5) lands behind this same command,
  which is why the surface takes JSON rather than per-verb flags.
- No verb reads `audit_log`, so an operator cannot review the audit
  trail through the only surface. Noticed while working around the
  lint, recorded here, and a contract question rather than a fix.
- `yeomna call` connects as `yeomna_app` always, which is right today
  and is where a future `--role` would have to argue against the verb
  layer's whole point.
