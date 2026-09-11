# PRD: The MCP Front End

Parent: `README.md`, the Yeomna Charter, sections 5.2 and 6.
Status: v0.5, 2026-09-11. Phases 1 and 2 built.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust, JSON,
and SQL keep their syntax.

## Revision History

| Version | Date | Change |
|---|---|---|
| 0.1 | 2026-09-10 | First draft. Unparks MCP for the stdio case only, on Todd's direction, and front-loads thirteen decisions plus one open ruling. |
| 0.2 | 2026-09-10 | Review pass. D7 gained `resultType` and the ssh-255 distinction, D10 became the fuller cancellation rule, and D12 and D13 are new: the request travels on stdin rather than argv, and the target owns which database a call reaches. The argv finding was a real defect. |
| 0.5 | 2026-09-11 | Built as spec 024. **D10 amended by a build finding**: end of input finishes in-flight work rather than cancelling it, because the first build's reading lost answers for calls that had already committed. Everything else built as decided. |
| 0.4 | 2026-09-11 | **R29 ruled by Todd: tool abstractions only, `sql` excluded by name.** D6 goes to 41 of 42 and the R29 section records the reasoning, including that the exclusion rests on section 6's sentence and on authorship rather than on a hazard, since R17a already makes the corpus structurally unreachable. The verb itself is untouched and stays reachable through `yeomna call` and the CLI. |
| 0.3 | 2026-09-10 | Second review pass. **D13 was wrong and is rewritten**: `QueryRequest` carries no `graph` field and `hybrid` reads the session's scope, so a session-scoped verb takes its graph from the target's config and not from the tool's arguments. D12 gained the stdin-close requirement, the caching contract is stated for the two cacheable operations, and the architecture diagram and Phase 2 example were still showing the argv form v0.2 had abolished. |

## Executive Summary

WeaverTools is developed on two machines, this server and a laptop, and
both need to reach the same knowledge graph. The charter already names
the answer: "A model on another machine reaches the store through a
front-end service on the far side of the socket, which is where an MCP
interface would live." This PRD builds that service.

It builds the cheap half of it. The expensive half is the actor, and the
whole shape of this PRD is chosen to avoid paying for it yet.

**The transport is stdio, not streamable HTTP** (D1). An MCP server on
stdio is a subprocess the client launches, so it binds no listener,
opens no port, and needs no charter amendment. The remote leg is ssh
(D2), which means a call from the laptop lands on this machine as a real
uid, and `crates/yeomna-verbs/src/actor.rs` names it from
`/proc/self/status` exactly as it does for a local call. Every audit row
stays kernel-named. Nothing about the three-layer seal changes and
nothing about the provenance model changes.

The streamable HTTP transport is the natural next step and it is
deferred rather than dropped (Phase 3), because it cannot be built
without ruling what an actor is when it is not a uid. That ruling pairs
with R22 and it is named here so it is not discovered later.

The second thing this PRD buys is documentation. The verb contract
already documents itself: all 42 `Verb` variants carry a doc comment, 48
lines of them, and 22 of the 30 request structs document fields. None of
it reaches a caller. MCP tool definitions carry a `description` per tool
and per property, and schemars lifts Rust doc comments into exactly those
fields. So the tool list is derived from the contract (D5) the same way
the CLI tree is, the doc comments become the reference documentation
they always were, and a verb added to the enum is callable the same day.

## Background and Context

### What exists

The verb layer is complete in all seven phases: 42 wire names in a
closed serde-tagged enum, `deny_unknown_fields` at both levels, one
envelope, a five-kind error taxonomy, and one audit row per call with
the outcome committed before the verb runs. Three surfaces reach it and
all three are the same layer underneath:

- `yeomna call <json>`, whose caller is a program.
- The contract-born CLI tree, where a wire name becomes a subcommand
  path with its dots as spaces (R21 D10).
- `yeomnad` over a Unix socket, naming its peer from `SO_PEERCRED` and
  refusing a uid this machine cannot name (R21 D6).

### Why MCP was parked, and what unparks it

Charter 5.2 parks MCP as a front-end concern and not current work. That
is a statement about order, not a prohibition, and the same paragraph
names MCP as the sanctioned place for a wire surface. Spec 021 carried a
do-not-reproduce item against `--mcp-*` flags on the CLI, and that item
stands: the objection was to a captured CLI declaring flags its own
record said were dropped, one of them naming a file that never existed.
A separate binary is not that (D9).

What unparks it is a real need with a date on it. Two machines, one
graph, and the laptop cannot hold a second copy: D10 forbids
`EmbeddingEndpoint` a network variant and the embedder is resident on
GPU 2 of this machine, so `query --hybrid` from the laptop has to
execute here whatever the transport is. The laptop is a thin caller in
every design.

### What the 2026-07-28 MCP revision changed, and why it helps

Two changes in the current revision bear directly on this build.

**The protocol core is stateless.** There is no connection-scoped
`initialize` handshake in the modern era. The protocol version, client
capabilities, and optional client identity ride in
`_meta.io.modelcontextprotocol/*` on every request, and `server/discover`
returns a `DiscoverResult` carrying `supportedVersions`. A stateless
server is a much smaller thing to build correctly, and it removes any
temptation to hold a verb session open across calls.

**The tool set must not vary per connection.** That forecloses a design
where the server curates tools per caller, which is the design that
would have drifted from the contract. All 42, always, in a deterministic
order (D6), which `WIRE_NAMES` supplies for free.

One smaller thing helps more than it looks: MCP tool names permit
letters, digits, underscore, hyphen, and dot. `graph.traverse` is a legal
tool name. The wire name transfers verbatim with no transformation at
all (D4), which is a better outcome than the CLI got.

### The one thing that is not free

Over a network there is no `SO_PEERCRED`. An HTTP MCP server is a local
process with its own uid, so every remote call would land in the audit
log as one service account, and section 6 calls that log a compliance
property rather than a convenience. OAuth 2.1 authenticates a client to
the server and does not fix this by itself: to keep the log honest the
authenticated subject has to reach the verb layer as the actor, which
means `Session` gains a way to be told who it is instead of asking the
kernel, and `actor.rs` exists to refuse precisely that.

The clean shape is an actor pair, the kernel-named local process plus an
authenticated remote subject, recorded as one row that reads "the front
end acting for todd@laptop". That is an `audit_log` schema change, and
T4 prices a schema change at drop, re-apply, template re-stamp, and
re-ingest. It is Phase 3 and it needs a ruling first.

## User Stories

- **As Claude Code on this machine**, I see 42 typed tools with
  descriptions and argument schemas, so I can call the graph without
  knowing a CLI and without a skill telling me a command line.
- **As Claude Code on the laptop**, I call the same 42 tools against the
  same graph, and the audit log on the appliance records me as todd
  rather than as a service account.
- **As the operator**, I add no port, no listener, and no credential to
  reach the appliance from a second machine, because ssh already exists
  and is already configured.
- **As an auditor**, every call through this surface is
  indistinguishable in the log from a call typed at the appliance's own
  keyboard, because it is one.
- **As a future reader of the contract**, the doc comment I write on a
  `Verb` variant is what a caller reads, so it is worth writing.

## Goals

1. A `yeomna-mcp` binary speaking MCP over stdio, serving all 42 verbs
   as tools.
2. Tool names, descriptions, and input schemas derived from `Verb` and
   its request structs, never written beside them.
3. A local target and an ssh target, chosen by launch argument.
4. Kernel-named actors preserved unchanged, proven by a test that reads
   the audit row a tool call leaves.
5. Zero changes to the verb layer's behavior, the schema, or the seal.

### Non-Goals

1. **No streamable HTTP transport.** Phase 3, blocked on the actor
   ruling. This is the biggest non-goal and the reason the rest is
   cheap.
2. **No OAuth, no authorization framework, no credential handling.** The
   stdio server authenticates nobody, because ssh and the kernel already
   did.
3. **No MCP resources and no MCP prompts.** Tools only. Resources over a
   corpus this size is its own design and the graph is already reachable
   as tools.
4. **No `--mcp-*` flags on `yeomna`.** Separate binary (D9).
5. **No second execution path.** The server shells out to `yeomna call`
   and holds no database connection (D3).
6. **No curated tool subset.** All 42 (D6).
7. **No new verb, no new request field, no schema version.** If this
   build wants one, that is a finding and it stops.
8. **No network listener anywhere**, which is the same non-goal as 1
   viewed from the seal's side.
9. **No caching of results.** `tools/list` may advertise a TTL because
   the tool list is static. Verb results are never cached.

## Feature Specifications

### Phase 1: The derived tool list and the stdio server

The crate, the schema derivation, the stdio loop, `server/discover`,
`tools/list`, `tools/call`, and the local target. This is spec 024.

The derivation is one call. `schema_for!(Verb)` over the
adjacently-tagged enum yields a `oneOf` with one entry per variant, each
carrying the variant's doc comment as its description and a reference to
its `args` schema. Walking that produces 42 tools with no table to
maintain, once the reference is resolved as D5 describes. A census test
asserts one tool per `WIRE_NAMES` entry and no others.

Error mapping is two-way and deliberate (D7). An unknown tool or a
malformed `tools/call` is a JSON-RPC error, because the model cannot fix
it. A verb refusal is a result with `isError: true` carrying the
envelope's message, because that is the case the spec says a model
should see so it can correct itself, and Yeomna's refusals are written
to be read.

### Phase 2: The ssh target

`--ssh <destination>` runs `ssh <destination> yeomna call -` and writes
the request to that process's stdin, per D12. Connection reuse is ssh's
`ControlMaster` and `ControlPersist`, configured in the user's own
`~/.ssh/config`, which is the inherit-do-not-author answer to spawn cost.

The laptop then runs `yeomna-mcp --ssh olympus` with no Yeomna install,
no config file, and no credential of its own.

### Phase 3: Deferred, and named so it is not discovered

The streamable HTTP transport with OAuth 2.1, resource indicators, and
RFC 9207 `iss` validation. Blocked on the actor ruling above, and it
carries a charter amendment with it: section 5.2 requires each network
opening be named in the charter and secured at the router and at the
machine before traffic is routed to the socket. GitHub is the first
named opening. This would be the second.

## Technical Architecture

```
Claude Code (laptop)                    olympus
  |                                       |
  | stdio (newline JSON-RPC)              |
  v                                       |
yeomna-mcp --ssh olympus  --- ssh --->  yeomna call -
  |                       request on stdin |  (real uid: todd)
  | links yeomna-verbs                     v
  | for schemas only                    yeomnad or embedded
                                           v
                                     Unix socket, port 5433
```

On this machine the ssh hop is absent and `yeomna-mcp --local` spawns
`yeomna call -` directly, writing the request to its stdin the same way.
Both legs invoke the same fixed command and end in the same binary
reaching the same socket, which is why there is one execution path and
not two, and why no request text is ever a command-line argument.

`crates/yeomna-mcp` depends on `yeomna-verbs` with a non-default
`schema` feature for the derivation, and on nothing else from the
workspace. It stays outside the no-SQL lint's allowlist and contains no
SQL.

### Resolved Design Decisions

**D1. stdio, not streamable HTTP.** The actor. stdio needs no listener,
no charter amendment, no authorization framework, and no schema change,
and it preserves kernel-named actors exactly. HTTP is Phase 3.

**D2. The remote leg is ssh, not a Yeomna transport.** Charter section 5
argues for standard tooling a buyer's IT staff already recognizes, and
"inherit, do not author" says take authentication from OpenSSH rather
than writing one. The decisive property is not convenience: ssh lands as
a real uid, so the audit row is honest for free.

**D3. Execution shells out to `yeomna call`.** The server holds no
database connection, links no store code, and has exactly one execution
path shared with every other surface. It is a translator. The
alternative, linking the verb layer, would make the MCP server a fourth
place a verb can be executed and a fourth place a session can be held.

**D4. Tool names are wire names verbatim.** MCP permits dots in tool
names, so `graph.traverse` needs no transformation. R21 D10 turned dots
into spaces because a shell subcommand path cannot hold a dot. That
constraint does not exist here and the workaround does not travel.

**D5. Schemas are derived, behind a feature.** `schema_for!(Verb)` and
nothing hand-written, because a tool table would be a second contract
and the CLI already settled that argument. The `JsonSchema` derives sit
behind a non-default `schema` feature on `yeomna-verbs` so the daemon
and the CLI do not carry schemars.

Probed against schemars 1 on 2026-09-10 before writing the spec, and the
rendering is what the decision assumed in every respect but one. The
`oneOf` carries one entry per variant, the variant doc comment arrives as
that entry's `description`, `verb` arrives as a `const`, field doc
comments arrive as property descriptions, `deny_unknown_fields` becomes
`additionalProperties: false`, serde defaults are carried as `default`,
and `Empty` renders as exactly the `{"type": "object",
"additionalProperties": false}` that the MCP revision recommends for a
tool taking no parameters.

The exception: `args` is a `$ref` into a `$defs` map at the root of the
whole enum schema, not an inlined subschema. So a tool's `inputSchema` is
not liftable straight out of its `oneOf` entry, and the derivation has to
resolve the reference and carry the `$defs` the request needs along with
it. This is a shape detail rather than a decision, and it is written down
because a builder who assumes an inlined schema produces 42 tools whose
`inputSchema` is a dangling reference, which no test of the tool *count*
would catch.

**D6. Tool abstractions only: 41 of the 42, with `sql` excluded by name.**
**Ruled by Todd 2026-09-11 (R29).** A model-controlled surface carries the
verbs that abstract over SQL and not the one that passes it through. The
charter sentence being honored is section 6's: "Nobody writes SQL, JSONB
path expressions, traversals, or search calls." The verb layer emits those
underneath, and a model composing a statement through a tool is something
writing SQL one layer above where that line was drawn.

This is a principle rather than a one-off. `sql` is the only passthrough
among the 42 today, so the rule names one verb, and a future verb that
hands raw text to the engine is excluded by the same rule without a new
ruling.

What did not change: **the verb is untouched.** `sql` stays in the
contract at 42 wire names, stays reachable through `yeomna call` and the
CLI tree, and keeps R17 and R17a's scoping. Only this surface omits it.
The exclusion is not a claim that the verb is unsafe. R17a already refuses
the appliance's own footing, templates, and any kg-pattern database by
catalog probe, running as a role with no grant on any KG table, so the
corpus was structurally unreachable already. The exclusion is about who
authors a statement, not about what the statement can touch.

Everything else stays as it was: no per-caller curation, since the
revision forbids a tool set that varies by connection, and `WIRE_NAMES`
order for the 41 that remain, which satisfies the deterministic-ordering
guidance for free.

**D7. Error mapping follows the exit code, and transport is not a verb.**
Exit 2 becomes a JSON-RPC error, because the caller or the machine was
wrong and no call was made. Exit 1 becomes a result with `isError: true`,
because a refusal is actionable feedback a model can correct against.
Exit 0 becomes a result whose `structuredContent` is the envelope and
whose text content is the same envelope serialized.

Two additions the first draft of this decision missed. **An ssh exit of
255 is the transport failing, not the appliance refusing**, and it is
distinguishable because `yeomna call` returns only 0, 1, or 2. Any other
unexpected status is reported as itself rather than folded into a verb
outcome, so a caller never debugs the wrong machine. And **every result
carries `resultType: "complete"`**, which the revision requires on all
results including the ones carrying `isError: true`. That field belongs to
the envelope MCP wraps rather than to anything Yeomna produces, which is
exactly why it is easy to omit, so it is set once in the code that writes
a result.

A refusal stays a result throughout. It never becomes a JSON-RPC error,
because Yeomna's refusals are written to be read and the revision says a
model should see them.

**D8. The target is a launch argument, not a config file.** R3 ruled
config-file for the appliance, and this is not the appliance. The MCP
client already keeps a config declaring how to launch its servers, and
putting the target there means one file rather than two that can
disagree. The laptop needs no `/etc/yeomna`.

**D9. A separate binary, not flags on `yeomna`.** Honors spec 021's
do-not-reproduce item, and keeps a front-end concern out of a surface
the charter says is the only surface.

**D10. Cancellation terminates what this process owns, and says so. End
of input does not.** `notifications/cancelled` means stop and send nothing
further for that id, and the local child is terminated and reaped.

**Amended 2026-09-11 by a build finding.** This decision first said the
same cleanup runs on stdin EOF, and dogfooding falsified it inside an
hour: a client that writes its requests and closes the stream lost every
answer whose call was still running, and a verb that had already
committed left a NULL audit outcome for work that succeeded. On EOF the
server finishes and answers what is in flight, then exits. Abandoning
committed work is the client's call to make explicitly and never
something to infer from a closed pipe, and the binding already gives the
client SIGTERM and SIGKILL if a call outlasts its patience. **A remote verb
already inside its transaction may still run to completion**, because ssh
does not forward signals without a tty. This is named rather than papered
over, and the existing audit design is what makes it visible: the row
commits before the verb runs, so an abandoned destructive call leaves a
NULL outcome naming its actor rather than no trace. The revision's own
wording is that a server SHOULD stop work as soon as practical, which is
best effort by construction.

**D11. Nothing but valid MCP messages on stdout.** The binding requires
it, and the hazard is specific: `yeomna call` prints its envelope to
stdout, and the CLI tree deliberately puts headers and rows on stdout
together. The child's stdout is captured, never inherited. All logging
goes to stderr, which the binding leaves free.

**D12. The request travels on stdin, never in argv.** Both targets invoke
the fixed form `yeomna call -` and write the request to the child's
stdin. For the local target this is tidiness. For ssh it is required:
**ssh joins its command arguments into one string and hands it to a shell
on the far side**, so a request carrying a quote, a backtick, a dollar
sign, or a semicolon would be interpreted there rather than delivered.
Request text is corpus-derived and caller-supplied, which makes argv an
injection path rather than a formatting choice. Using one form for both
targets also removes the argv length ceiling and leaves one code path to
get right.

**Exactly one request per child, and its stdin is closed after writing.**
`yeomna call -` reads a request from stdin, so a child whose stdin stays
open waits for more input and the call never returns. The close is the
signal that the request is complete, which makes it a correctness
requirement rather than tidiness, and it is the kind of omission that
hangs rather than errors.

**D13. The target owns the database and the session graph. The contract
owns the rest.** The first draft of this decision said the graph is
always a request field, and that is wrong for a class of verbs. Both
sources are real and which one applies is the contract's business:

- **Verbs whose request struct carries a `graph` field** name it in the
  tool's arguments. `graph.traverse`, `graph.neighbors`, and the rest of
  that family work this way, and the derived `inputSchema` already
  exposes the field.
- **Verbs that read the session's scope** get it from the config on the
  machine where `yeomna call` runs. `QueryRequest` has no `graph` field
  at all, and `hybrid` reads `s.graph()` and refuses without one, so
  `query --hybrid` is in this class. The config key exists already and is
  documented as "the graph a session starts scoped to when the caller
  names none".

So the target supplies `database` and, for session-scoped verbs, `graph`.
Reaching the WeaverTools KG for a hybrid query needs a config naming both.
The MCP server passes neither of its own and adds no override, because an
override would make this front end a second place scope is decided and
would need an argument no request struct defines.

**The consequence worth stating: one server instance is scoped to one
graph for session-scoped verbs.** Two graphs means two entries in the MCP
client's config, which composes with D8 rather than fighting it, since
each entry can carry its own `YEOMNA_CONFIG`. This is a limit of the
design and not a defect, and naming it here is cheaper than discovering
it when a second graph is wanted.

### R29, ruled: `sql` does not belong on a model-controlled surface

**Ruled by Todd 2026-09-11: no `sql` on the model-controlled surface, tool
abstractions only.** Recorded with the reasoning that produced it, because
the narrow part is what makes the ruling worth keeping.

MCP sorts its primitives by who chooses. Tools are model-controlled, which
the specification defines as the model discovering and invoking them on its
own from context. That is the difference from the CLI, and it is a
difference about authorship rather than about permission: when a person
types a statement they decided those exact bytes should run, and when a
model calls the tool it composes them. The audit row names the same actor
either way and stays true, while what the actor authored thins from a
statement to an intent. That is the third place this theme has come up,
after R22 and the actor-over-a-network question.

**The blast radius was already narrow, and the ruling is not about blast
radius.** `crates/yeomna-verbs/src/sql.rs` refuses the current database,
`yeomna`, and `postgres` as the appliance's own footing, refuses templates,
and refuses any kg-pattern database by probing the catalog for five
signature tables, erring toward refusal so structure decides and a
hand-stamped graph is protected like a generated one. Statements run as
`yeomna_provision` on a per-call connection that holds no grant on any KG
table, and the drop is the reset. The corpus is structurally unreachable.

So the ruling rests on section 6's sentence and on authorship, not on a
hazard R17a left open. Todd's phrasing was "for now", which is what the
next paragraph is for.

**What would reopen it.** A caller that needs plain-database utility
through this surface and cannot reach a shell, or a client-side
confirmation property the appliance can verify rather than hope for. The
present answer relies on neither, which is why it is cheap to hold: the
exclusion is a filter over `WIRE_NAMES`, so reversing it is one line and
re-including a verb costs nothing that removing it did not already pay.

## Testing Strategy

- **Census, and it must assert identity rather than only count.** 41
  tools, one per `WIRE_NAMES` entry except `sql`, in contract order. **A
  count of 41 is not sufficient on its own**, because it passes if some
  other verb went missing while `sql` was present. The test names `sql` as
  the expected absence and asserts every other wire name is present, which
  is the same lesson FR3a learned: a count cannot see an identity problem.
- **The exclusion covers dispatch and not only the list.** A `tools/call`
  naming `sql` is an unknown tool, tested by calling it directly. A server
  that filters its list from the enum and builds its dispatch from the
  whole enum would leave a tool nobody advertises and anybody can call,
  which is the side-door shape the charter's T3 paragraph exists to
  forbid.
- **Descriptions are present.** Every tool carries a non-empty
  description, which holds today because all 42 variants are documented
  and turns a future undocumented variant into a failing test. The real
  gap is one level down: 8 of the 30 request structs document no field,
  so the same pass reports which properties reach a caller with no
  description. That report is work, recorded rather than passed over.
- **Schemas are real JSON Schema objects**, and a verb taking nothing
  emits `{"type": "object", "additionalProperties": false}` rather than
  `null`.
- **stdout purity.** A test drives the server over pipes, exercises a
  tool call whose child writes a table, and asserts every stdout line
  parses as JSON-RPC. This is D11 made mechanical.
- **The actor survives.** A gated test calls a verb through the server
  and reads the audit row it left, asserting the actor is the same name
  an identical `yeomna call` produces. This is the claim the whole
  design rests on and it gets checked by machine.
- **Error mapping.** A refusal arrives as `isError: true` with the
  envelope's message readable, and an unknown tool arrives as a
  JSON-RPC error.
- **Cancellation.** A cancelled call terminates and reaps its child and
  sends nothing further for that id, and stdin EOF does the same for every
  in-flight call.
- **Protocol conformance, by transcript rather than by reading.**
  `resultType` present on every result, the two required `_meta` fields
  enforced with `-32602`, and `-32021` and `-32022` used only with their
  specified meanings.
- **Caching hints where the revision requires them and nowhere else.**
  `server/discover` and `tools/list` carry `ttlMs` as an integer at or
  above zero and `cacheScope: "public"`, public because the tool list is
  derived at compile time and is identical for every caller. `tools/call`
  is not a cacheable operation, so a verb result carries no caching hint
  at all, which is the same claim as non-goal 9 made in the protocol's
  own vocabulary. `listChanged` is false, because a list derived from a
  compiled-in enum cannot change while the process runs.
- **No shell ever sees request bytes as syntax.** A request whose
  arguments carry `'`, `"`, backtick, `$(`, `;`, and a newline arrives at
  the verb byte-identical over both targets. This is the test whose
  absence would be silent on every happy path.
- The workspace gate three times green with the cluster up, clippy zero,
  fmt clean, and the no-SQL lint still passing with `yeomna-mcp` outside
  its allowlist.

## Risk Assessment

| Risk | Severity | Mitigation |
|---|---|---|
| A schemars version bump changes how an adjacently-tagged enum renders, silently reshaping 42 tools | low, was medium | Measured on schemars 1 before the spec was written, recorded under D5, and pinned in the workspace manifest. A shape-assertion test fails loudly rather than falling back, so a bump is a test failure and not a malformed tool list. |
| The `$defs` reference is resolved wrongly, so a tool's `inputSchema` points at nothing | medium | FR3a: every `$ref` in every tool resolves inside that same tool. This is the failure the tool count cannot see, which is why it gets its own requirement. |
| A doc comment becomes a caller-visible string, so a careless comment becomes bad documentation | low | The description test finds absent ones. The 125-violation editorial backlog now has a second reason to be swept, since some of those violations are in doc comments that would ship as tool descriptions. |
| ssh spawn cost per call | low | ControlMaster in the user's ssh config. Measured in Phase 2 and reported rather than assumed. |
| A long ingest exceeds the MCP client's timeout | medium | R21 D1 already rules that long verbs block. Cancellation is implemented (D10) and the NULL-outcome row makes an abandoned call visible. |
| Adding schemars to the contract crate spreads a dependency into the layer that must stay small | low | Non-default feature, enabled only by `yeomna-mcp`. |
| Request text reaching a remote shell through ssh's argument joining | high, mitigated | D12 makes stdin the only channel and FR15 tests the metacharacters directly. This was a real defect in the first draft of the spec, which had said to pass the request as a single argument. |
| A cancelled destructive verb keeps running on the far side | medium | D10 states the limit rather than hiding it, and the pre-committed audit row makes an abandoned call visible with its actor. |
| This is the first front-end component and front-end scope has no precedent here | medium | The non-goals list is long on purpose, and Phase 3 is named rather than left as a direction of travel. |

## Timeline

Phase 1 and Phase 2 are one spec and one PR, because Phase 2 is a second
target for a spawn that already exists. Phase 3 is not scheduled and
does not start before the actor ruling and R22 are settled together.
