# Specification: 024 The stdio MCP Server

Parent PRD: `docs/PRD-mcp-front-end.md` v0.4, Phases 1 and 2.
Owner: epic #21 (the holes ledger) as a new surface rather than a hole,
since no verb refuses for want of this. Ruled by that PRD's D1 through
D13, and by **R29, ruled by Todd 2026-09-11: tool abstractions only, so
`sql` is excluded from this surface by name.** The verb itself is
untouched and stays reachable through `yeomna call` and the CLI tree.
Status: built 2026-09-11, with one amendment recorded in Task Scope from
a build finding about stdin EOF. Revised twice on 2026-09-10 for CodeRabbit's
protocol and transport findings (round one: six applied, one referred to
Todd, round two: four applied, one of which corrected a wrong claim about
where a session-scoped verb gets its graph), then again for R29's ruling.

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

The streamable HTTP transport is not in scope and is not a later phase of
this product either. A transport that authenticates remote callers has to
remember them, and remembering is a database, which "one store, one
engine" puts on the far side of the socket with the front end that owns
it. That is a separate product calling this one, and an earlier draft was
wrong to schedule it here.

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
- Per-request metadata validation, because the protocol core is stateless
  and nothing may be inferred from an earlier request on the same
  connection. `_meta.io.modelcontextprotocol/protocolVersion` and
  `_meta.io.modelcontextprotocol/clientCapabilities` are both required on
  every request, `clientInfo` is not. A request missing a required field
  is rejected with `-32602`, an unsupported version with
  `UnsupportedProtocolVersion` (`-32022`), and a request needing a
  capability the client did not declare with
  `MissingRequiredClientCapability` (`-32021`) carrying
  `data.requiredCapabilities`.
- `server/discover` returning a `DiscoverResult` with `supportedVersions`
  and the `tools` capability, since the modern era has no `initialize`
  handshake.
- **Build finding, 2026-09-11, amending this spec: the server must speak
  both eras.** As written, this spec was modern-only, and a modern-only
  server is unreachable from a client that opens with an `initialize`
  handshake. That is not hypothetical: Claude Code 2.1.268 opened that way
  and the registration failed with `-32602: params._meta is required`,
  because metadata was checked before the method was even looked at. The
  specification's own compatibility matrix names the cell, legacy client
  against modern server, and says such a client has "no fall-forward
  mechanism". A dual-era server is explicitly permitted and it is the only
  thing that makes this surface usable. So: `initialize` is answered with
  an `InitializeResult`, `notifications/initialized` is accepted silently,
  `ping` is answered in both eras, and the era selected by a handshake is
  remembered for the life of the process, which is what that revision
  scopes it to. Metadata is required in the modern era and not sent in the
  older one, and the modern-only fields (`resultType`, the per-response
  server identity, the caching hints) are not sent into the older era at
  all.
- `resultType` on every result. The revision requires it, `"complete"` is
  what this server returns, and it is easy to omit because the field
  belongs to the envelope MCP wraps rather than to anything Yeomna
  produces. Results also carry
  `_meta.io.modelcontextprotocol/serverInfo`, which the revision asks for.
- **The `sql` exclusion (R29, ruled 2026-09-11).** The derivation walks
  `WIRE_NAMES` and filters `sql` out, through a named constant carrying the
  reason rather than a bare string in a predicate, so a reader finds the
  ruling from the code. The exclusion applies to **both the tool list and
  the dispatch**: a `tools/call` naming `sql` is an unknown tool. A server
  that filtered the list and dispatched from the whole enum would ship a
  tool nobody advertises and anybody can call, which is the side door the
  charter's T3 paragraph forbids. The verb itself is untouched and stays
  reachable through `yeomna call` and the CLI tree.
- `tools/list` returning the 41 in `WIRE_NAMES` order, and the caching
  hints the revision requires on cacheable results. `server/discover` and
  `tools/list` both carry `ttlMs`, an integer at or above zero, and
  `cacheScope`, which is `"public"` here because the list is derived at
  compile time and is identical for every caller. **`tools/call` is not a
  cacheable operation**, so a verb result carries no `ttlMs` and no
  `cacheScope`. `listChanged` is false, since a list derived from a
  compiled-in enum cannot change while the process runs.
- `tools/call` building `{"verb": name, "args": arguments}`, spawning the
  target, and mapping the exit code per D7.
- Two targets, both invoking the fixed remote form `yeomna call -` and
  writing the request to the child's stdin: `--local` spawning
  `yeomna call -` directly, and `--ssh <destination>` spawning
  `ssh <destination> yeomna call -`. **The request never travels in
  argv.** ssh joins its command arguments into one string and hands it to
  a shell on the far side, so a request carrying a quote, a backtick, a
  dollar sign, or a semicolon would be interpreted there rather than
  delivered. Request text is corpus-derived and caller-supplied, which
  makes argv an injection path and stdin the only correct channel.
  **Exactly one request is written and then stdin is closed**, because
  `yeomna call -` reads until end of input and a child whose stdin stays
  open waits rather than answering. The close is how the request is
  terminated, so omitting it hangs the call instead of failing it.
- Transport failure distinguished from verb failure. `yeomna call` exits
  only 0, 1, or 2, so an ssh exit of 255 is ssh's own failure and every
  other unexpected status is reported as itself rather than mapped onto a
  verb outcome.
- `notifications/cancelled`: terminate and reap the in-flight child and
  send nothing further for that id.
- **Build finding, 2026-09-11, amending this spec.** The draft above said
  the same cleanup runs on stdin EOF, and the first build did that.
  Dogfooding falsified it: a client that writes its requests and closes
  the stream loses every answer whose call is still running, and a verb
  that had already committed leaves a NULL audit outcome for work that
  succeeded. **On EOF the server finishes and answers what is already in
  flight, then exits.** Abandoning committed work is the client's call to
  make explicitly through `notifications/cancelled` and never something to
  infer from a closed pipe. Prompt exit is still honored, since nothing
  new is accepted and the binding gives the client SIGTERM and SIGKILL as
  its backstop.

## Out of Scope

- The streamable HTTP transport, OAuth, and any credential handling. Not
  deferred: a front end that authenticates callers holds state, and a
  second store does not live in this box.
- MCP resources and MCP prompts. Tools only.
- Any flag on the `yeomna` binary.
- Any second execution path. The server never links `yeomna-store`,
  never opens a database connection, and emits no SQL.
- Any new verb, request field, or schema version. Wanting one is a
  finding that stops the build.
- Caching verb results. The tool list may carry a TTL. Results never do.
- Curating the tool set per caller. The one exclusion is `sql` by name
  under R29, applied identically for every caller, since the revision
  forbids a tool set that varies by connection.

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
- **FR2** `tools/list` returns exactly 41 tools, one per `WIRE_NAMES`
  entry except `sql`, in that order, each with a non-empty description and
  an `inputSchema` that is a valid JSON Schema object. The description
  check passes today because all 42 variants are documented, and its job is
  to fail the day one is not.
- **FR2a** The census names the absence rather than counting to it. The
  test asserts `sql` is the one wire name missing and that every other is
  present. **A count of 41 alone is not sufficient**, because it passes if
  another verb went missing while `sql` was present, which is FR3a's lesson
  in a second place: a count cannot see an identity problem.
- **FR2b** `tools/call` with `name` of `sql` is a JSON-RPC error for an
  unknown tool, proven by calling it directly rather than inferred from its
  absence in the list.
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
- **FR11** stdin EOF stops accepting new requests, finishes and answers
  what is already in flight, and then exits. It does not cancel work
  (see the build finding in Task Scope).
- **FR12** Exactly one of `--local` and `--ssh` is required. Neither, or
  both, is a usage error.
- **FR13** Every result carries `resultType: "complete"`, including
  `server/discover`, `tools/list`, a successful `tools/call`, and a
  `tools/call` that returns `isError: true`. A refusal is a result and so
  it carries `resultType` too.
- **FR14a** An `initialize` request is answered with an
  `InitializeResult` naming a negotiated version, the `tools` capability,
  and this server's identity. A version this server knows is echoed back,
  and one it does not know gets its newest handshake version as a
  counter-offer rather than a refusal, which is what the older lifecycle
  asks for. Afterwards every request works with no metadata on any of them.
- **FR14b** The older era is sent no field belonging to the newer one: no
  `resultType`, no per-response server identity, no caching hints.
- **FR14c** `ping` is answered in both eras, because its absence is a hang
  rather than an error.
- **FR14** In the modern era, a request missing `protocolVersion` or `clientCapabilities`,
  or carrying either malformed, is rejected with `-32602`. A version this
  server does not support is rejected with `-32022` listing what it does
  support. **Build finding:** the `-32021` case has no trigger on this
  surface and the code therefore has no path that raises it. None of
  `server/discover`, `tools/list`, or `tools/call` needs anything of the
  client, so the requirement is vacuous here and is recorded as vacuous
  rather than implemented as an unreachable branch. It returns with the
  first operation that needs a client capability.
- **FR15** The request reaches the child on stdin and is byte-identical
  to what the server built, for both targets, including requests whose
  text contains `'`, `"`, backtick, `$(`, `;`, and a newline. No shell on
  either side of the ssh hop sees request bytes as syntax.
- **FR16** An ssh exit status of 255 is reported as a transport failure
  naming the destination, not as a verb refusal. Statuses other than 0,
  1, 2, and 255 are reported as unexpected, quoting the status.
- **FR17** `server/discover` and `tools/list` each carry `ttlMs` as an
  integer at or above zero and `cacheScope` equal to `"public"`. A
  `tools/call` result carries neither field, because it is not a cacheable
  operation. The `tools` capability advertises `listChanged: false`.
- **FR18** The child receives exactly one request on stdin and then end of
  input, for both targets, so `yeomna call -` returns rather than waiting.
  A test asserts the call completes without a timeout, since the symptom
  of omitting the close is a hang and not an error.

## Edge Cases

- **EC-1** A child that writes nothing and exits nonzero: reported as a
  tool execution error naming the exit code, never as an empty success.
  Spec 023's review found this exact shape in a test and it is a rule
  here.
- **EC-2** A child whose stdout is not an envelope, for example a target
  where `yeomna` is absent or a different version: a JSON-RPC error
  quoting what arrived, truncated, because a parse failure that hides
  the bytes is unfixable from the outside.
- **EC-3** `ssh` itself failing (host down, key refused, host key
  changed): ssh exits 255, which `yeomna call` never returns, so the two
  are distinguishable in practice and the report names the destination
  rather than the verb. A caller that cannot tell transport from
  appliance debugs the wrong machine. The one honest limit: 255 is
  unambiguous because of what `yeomna call` chooses to return and not
  because ssh reserves it, so the mapping cites the CLI's exit codes as
  its warrant.
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
- **EC-9** A request whose arguments carry shell metacharacters, for
  example a `query` term of `"; rm -rf /"` or a document body containing
  `$(id)`: transmitted unchanged on stdin and answered as an ordinary
  query. Tested rather than reasoned about, because the failure would be
  silent on the happy path and catastrophic once.
- **EC-10** A client that omits `_meta` entirely, or sends
  `protocolVersion` as a number: `-32602`, and the connection stays
  usable, because the protocol is stateless and one bad request is not a
  broken session.
- **EC-11** A cancelled call whose remote verb is already inside its
  transaction: the local child is terminated and reaped and
  nothing further is sent for that id, and the remote verb may still run
  to completion. ssh does not forward signals without a tty, and closing
  the channel leaves the far side to notice. This is stated rather than
  papered over, and the audit row is what makes it visible: the row
  commits before the verb runs, so an abandoned destructive call leaves a
  NULL outcome naming its actor rather than no trace. MCP's own wording
  is that a server SHOULD stop work as soon as practical, which is a
  best-effort obligation and is met by terminating what this process
  owns.

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
- DO write the request to the child's stdin and invoke the fixed form
  `yeomna call -` for both targets. One code path, no argv length
  ceiling, and no shell on the far side of ssh that could read request
  bytes as syntax.
- DO set `resultType` once, in the code that writes a result, so no
  response path can forget it.
- DO run the gate three times with the cluster up.

DON'T:

- DON'T link `yeomna-verbs`' execution path, open a database
  connection, or emit SQL. This crate translates.
- DON'T add a flag to `yeomna`.
- DON'T write a tool table by hand.
- DON'T hold state between calls. The protocol core is stateless and a
  held session would be a fourth place a session lives.
- DON'T report a refusal as a JSON-RPC error, a transport failure as a
  refusal, or an ssh 255 as an appliance problem.
- DON'T interpolate any part of a request into a command line, an `ssh`
  argument, or a shell string, under any circumstance.
- DON'T claim a cancelled call stopped remote work. Terminate what this
  process owns and say what the audit row will show.
- DON'T let the ssh target default to a host. An absent `--ssh` value is
  a usage error.

## Success Criteria

1. 41 tools, derived, one per wire name except `sql`, each with a
   description that came from the contract's own doc comment and an
   `inputSchema` whose references all resolve. `sql` is absent from the
   list and unreachable through dispatch, and the test names it rather
   than counting to it.
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
- Tests cover FR1 through FR18, FR2a, FR2b, FR3a, and FR14a through
  FR14c, and EC-1 through EC-11.
- An end-to-end check from the laptop: `yeomna-mcp --ssh olympus`, a
  `tools/list`, a `query --hybrid` tool call against the `weavertools`
  graph, and the audit row read back on the appliance. **Which database
  that call lands in is the target's business, not this server's.**
  `yeomna call` resolves the database from the config on the machine
  where it runs, so reaching the WeaverTools KG needs a config there
  naming `database = "weavertools"`, reached by `/etc/yeomna/yeomna.toml`
  or `YEOMNA_CONFIG`. The MCP server passes no database and no graph of
  its own: the graph is a request field the contract already defines for
  the verbs that take one, and it arrives in the tool's arguments like
  any other. There is no implicit override and none is added.
- Protocol conformance checked against the 2026-07-28 revision by
  transcript rather than by reading: `resultType` present on every
  result, required `_meta` fields enforced, and the three MCP error codes
  used only with their specified meanings.
- The description check reports which variants and fields lack doc
  comments, and that list is recorded in the review notes as work rather
  than silently passed.
