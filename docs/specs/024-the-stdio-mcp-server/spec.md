# Specification: 024 The stdio MCP Server

Parent PRD: `docs/PRD-mcp-front-end.md` v0.1, Phases 1 and 2.
Owner: epic #21 (the holes ledger) as a new surface rather than a hole,
since no verb refuses for want of this. Ruled by that PRD's D1 through
D11. **R29 proposed** on whether `sql` belongs on a model-controlled
surface.
Status: draft, 2026-09-10.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust, JSON,
and SQL keep their syntax.

---

## Overview

WeaverTools is developed on two machines and both need the same graph.
This spec builds the front end charter section 5.2 names, in the one
shape that costs nothing: an MCP server on stdio, which the client
launches as a subprocess, with ssh as the remote leg.

The shape is chosen for a property rather than for convenience. A call
from the laptop arrives on this machine as a real uid, so
`crates/yeomna-verbs/src/actor.rs` names the actor from
`/proc/self/status` the way it already does, and every audit row stays
kernel-named. No listener opens, no credential is authored, no schema
version is spent, and the three-layer seal is untouched.

The tool list is derived from the contract, which is the third time this
argument has been settled the same way. `schema_for!(Verb)` over the
adjacently-tagged enum yields one `oneOf` entry per variant carrying the
variant's doc comment and its `args` subschema, so 42 tools fall out of
one call with no table beside them. All 42 variants carry a doc comment
already, 48 lines of them, and 22 of the 30 request structs document
their fields. That becomes the reference documentation it has always been
and has never reached a caller.

## What this is not, and why the record matters

Spec 021 carried a do-not-reproduce item reading "No `--mcp-*` flags.
MCP is parked, the capture declared flags its own record said were
dropped, and one of them named a file that never existed." That item
stands and this spec does not touch it. The objection was to a captured
CLI advertising a surface that did not exist. This is a separate binary
with no flags on `yeomna` at all (D9), built because two machines need
one graph, and the charter paragraph that parks MCP is the same one that
names MCP as where a wire surface belongs.

The streamable HTTP transport is not in scope and not merely unbuilt: it
cannot be built until an actor over a network is ruled, because there is
no `SO_PEERCRED` on a network socket and one service account in the audit
log is not the record section 6 promises.

## Task Scope

- A new crate, `crates/yeomna-mcp`, binary `yeomna-mcp`, twelfth in the
  workspace.
- A non-default `schema` feature on `yeomna-verbs` adding `JsonSchema`
  derives to `Verb` and its 30 request structs, so the daemon and the CLI
  do not carry schemars.
- The derivation: `schema_for!(Verb)` walked into 42 tool definitions,
  each with the wire name verbatim as its name (D4), the variant doc
  comment as its description, and the `args` schema as its `inputSchema`.
  **The `args` schema arrives as a `$ref` into the enum schema's root
  `$defs`, measured 2026-09-10**, so the derivation resolves the
  reference and carries the `$defs` that request needs into the tool's
  own `inputSchema`. A tool whose `inputSchema` is a reference with
  nowhere to resolve is still a well-formed tool object and still counts
  as 42, so the count test cannot find this and a resolution test has to.
- The stdio loop per the 2026-07-28 binding: newline-delimited JSON-RPC
  on stdin and stdout, no embedded newlines, nothing but valid MCP
  messages on stdout, logging to stderr, exit promptly on stdin EOF.
- `server/discover` returning a `DiscoverResult` with `supportedVersions`
  and the `tools` capability, since the modern era has no `initialize`
  handshake and the protocol version rides in
  `_meta.io.modelcontextprotocol/protocolVersion` per request.
- `tools/list` returning all 42 in `WIRE_NAMES` order, with a TTL
  advertised because the list is static.
- `tools/call` building `{"verb": name, "args": arguments}`, spawning the
  target, and mapping the exit code per D7.
- Two targets: `--local` spawning `yeomna call`, and
  `--ssh <destination>` spawning `ssh <destination> yeomna call`.
- `notifications/cancelled`: kill the child, send nothing further for
  that id.

## Out of Scope

- The streamable HTTP transport, OAuth, and any credential handling
  (Phase 3, blocked on the actor ruling).
- MCP resources and MCP prompts. Tools only.
- Any flag on the `yeomna` binary.
- Any second execution path. The server never links `yeomna-store`,
  never opens a database connection, and emits no SQL.
- Any new verb, request field, or schema version. Wanting one is a
  finding that stops the build.
- Caching verb results. The tool list may carry a TTL. Results never do.
- Curating the tool set (D6). See R29.

## Files to Modify

- `crates/yeomna-mcp/Cargo.toml`, `src/main.rs`, `src/tools.rs`,
  `src/target.rs`, `src/rpc.rs` (all new).
- `crates/yeomna-verbs/Cargo.toml`: the `schema` feature.
- `crates/yeomna-verbs/src/verb.rs`: `cfg_attr` derives on the enum and
  the 30 request structs.
- `Cargo.toml`: the member and the schemars pin.
- `crates/yeomna-mcp/tests/mcp.rs` (new): the census and the rest.
- `docs/holes.md`, `CLAUDE.md`, epic #21.

## Files to Reference

- `crates/yeomna-verbs/src/verb.rs`: `Verb`, `WIRE_NAMES`, the request
  structs, and the doc comments that become descriptions.
- `crates/yeomna-verbs/src/actor.rs`: what the actor test asserts about.
- `crates/yeomna-cli/src/main.rs`: `call`'s exit codes, which D7 maps.
- `crates/yeomna-daemon/src/server.rs`: the precedent for a surface that
  translates rather than executes.

## Functional Requirements

- **FR1** `yeomna-mcp --local` answers `server/discover` with the
  `tools` capability and a `supportedVersions` list including the
  revision it implements.
- **FR2** `tools/list` returns exactly 42 tools, one per `WIRE_NAMES`
  entry, in that order, each with a non-empty description and an
  `inputSchema` that is a valid JSON Schema object. The description
  check passes today because all 42 variants are documented, and its
  job is to fail the day one is not.
- **FR3** A verb taking no arguments emits
  `{"type": "object", "additionalProperties": false}`, never `null`.
  `Empty` already renders exactly this, so the requirement is that it
  stays true rather than that it be built.
- **FR3a** Every `$ref` in every tool's `inputSchema` resolves inside
  that same `inputSchema`. No tool refers to a definition that did not
  travel with it.
- **FR4** `tools/call` on `graph.neighbors` with valid arguments returns
  a result whose `structuredContent` is the envelope and whose text
  content is that envelope serialized.
- **FR5** A verb refusal returns `isError: true` with the envelope's
  message readable in the text content, and the process exits 0. The
  refusal is not a JSON-RPC error.
- **FR6** An unknown tool name, or arguments the contract rejects for
  shape, returns a JSON-RPC error naming the problem.
- **FR7** Nothing that is not a valid MCP message reaches stdout, under
  every path including a child that writes a table and a child that
  writes to stderr.
- **FR8** The actor an MCP tool call leaves in the audit log equals the
  actor an identical `yeomna call` leaves.
- **FR9** `--ssh <destination>` reaches the same contract with the same
  results, and the audit row on the remote names the ssh login rather
  than a service account.
- **FR10** `notifications/cancelled` for an in-flight id kills the child
  and produces no further message for that id.
- **FR11** stdin EOF exits the process promptly.
- **FR12** Exactly one of `--local` and `--ssh` is required. Neither, or
  both, is a usage error.

## Edge Cases

- **EC-1** A child that writes nothing and exits nonzero: reported as a
  tool execution error naming the exit code, never as an empty success.
  Spec 023's review found this exact shape in a test and it is a rule
  here.
- **EC-2** A child whose stdout is not an envelope, for example a target
  where `yeomna` is absent or a different version: a JSON-RPC error
  quoting what arrived, truncated, because a parse failure that hides
  the bytes is unfixable from the outside.
- **EC-3** `ssh` itself failing (host down, key refused): distinguished
  from a verb failure, because one is the transport and one is the
  appliance, and a caller that cannot tell them apart debugs the wrong
  machine.
- **EC-4** An envelope containing a newline in a message: the JSON-RPC
  line is a single JSON string, so the newline is escaped by
  serialization. Asserted rather than assumed, since the binding forbids
  embedded newlines and the corpus supplies arbitrary text.
- **EC-5** Corpus text carrying control characters, which
  `crates/yeomna-cli/src/render.rs` sanitizes at the render boundary:
  the MCP path carries the envelope rather than the rendered table, so
  the sanitizer is not in this path and the JSON string escapes what it
  must. Named so the absence is deliberate.
- **EC-6** Two concurrent `tools/call` requests: each spawns its own
  child, its own connection, and its own session, so the appliance's
  per-session serialization is unaffected and each call gets its own
  audit row.
- **EC-7** A very large result, for example an unbounded `list`: bounded
  by the verb's own row caps, which already exist. The server adds no
  cap of its own and does not truncate, because a truncated result that
  says nothing is the failure mode D5 of the embedder PRD exists to
  forbid.
- **EC-8** A property whose field has no doc comment: the tool is still
  valid and the property reaches the caller undescribed. 8 of the 30
  request structs are in that state today, so this is reported as a list
  rather than treated as a build failure, and the list is the work it
  names. A missing *variant* doc comment is the stricter case and does
  fail FR2.

## Implementation Notes

DO:

- DO derive from `schema_for!(Verb)` and assert the shape you depend on,
  so a schemars change is a loud test failure rather than a quiet
  malformed tool. The shape as measured is in the PRD under D5.
- DO resolve `args` through the root `$defs` and carry the definitions a
  request needs into its tool. Copying the whole `$defs` map into every
  tool is acceptable and correct. Inlining by hand is not, because
  request structs share types and a hand-inliner will diverge from what
  serde accepts.
- DO capture the child's stdout and stderr. Inheriting stdout would put
  a table on the MCP channel, which is D11 and the binding's hardest
  rule.
- DO write every log line to stderr.
- DO let ssh handle connection reuse through the user's own
  `~/.ssh/config`. `ControlMaster` and `ControlPersist` exist and are
  not this crate's business.
- DO pass the request to the child as a single argument, and prefer
  stdin (`yeomna call -`) if argument length is ever a question, since
  the CLI already supports it.
- DO run the gate three times with the cluster up.

DON'T:

- DON'T link `yeomna-verbs`' execution path, open a database
  connection, or emit SQL. This crate translates.
- DON'T add a flag to `yeomna`.
- DON'T write a tool table by hand.
- DON'T hold state between calls. The protocol core is stateless and a
  held session would be a fourth place a session lives.
- DON'T report a refusal as a JSON-RPC error, or a transport failure as
  a refusal.
- DON'T let the ssh target default to a host. An absent `--ssh` value is
  a usage error.

## Success Criteria

1. 42 tools, derived, one per wire name, each with a description that
   came from the contract's own doc comment and an `inputSchema` whose
   references all resolve.
2. A tool call from the laptop leaves an audit row on the appliance
   naming a person, proven by reading the row.
3. Nothing but MCP messages on stdout, proven by driving the server over
   pipes.
4. No listener, no credential, no schema version, no charter amendment.
5. Gate three times green with cluster tests running, clippy zero, fmt
   clean, no-SQL lint passing with `yeomna-mcp` outside its allowlist.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR12, FR3a included, and EC-1 through EC-8.
- An end-to-end check from the laptop: `yeomna-mcp --ssh olympus`, a
  `tools/list`, a `query --hybrid` tool call against the `weavertools`
  graph, and the audit row read back on the appliance.
- The description check reports which variants and fields lack doc
  comments, and that list is recorded in the review notes as work rather
  than silently passed.
