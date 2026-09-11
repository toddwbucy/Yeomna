# Review notes: 024 The stdio MCP Server

PR #57, merged 2026-09-11. Documents only, so there was no local
`/code-review` pass: that step reviews code and this branch had none.
What stood in for it, and what CodeRabbit found across two exchanges,
is recorded here because three of the findings were defects rather than
omissions.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

---

## What was checked before the PR opened

- Editorial rules mechanically on both files.
- Every factual claim about the contract re-derived from `verb.rs`
  rather than carried from the conversation that produced the spec.
  This caught two counts that had been stated wrongly: all 42 `Verb`
  variants are documented, so tool descriptions are covered, and the
  real gap is that 8 of the 30 request structs document no field.
- **D5's mechanism falsified against a real schemars build** in a
  scratch crate rather than assumed. This is what produced the `$ref`
  finding below, before any reviewer saw the spec.

## The finding the pre-PR probe produced

`schema_for!(Verb)` renders the adjacently-tagged enum as a `oneOf` with
the variant doc comment as each entry's `description`, `verb` as a
`const`, field doc comments as property descriptions,
`deny_unknown_fields` as `additionalProperties: false`, and `Empty` as
exactly the `{"type": "object", "additionalProperties": false}` the MCP
revision recommends for a tool taking no parameters.

**The exception: `args` is a `$ref` into a root-level `$defs`.** A tool's
`inputSchema` is therefore not liftable straight out of its `oneOf`
entry. A builder who assumes an inlined schema ships 42 tools whose
`inputSchema` is a dangling reference, and **no test of the tool count
would catch it**, because a dangling reference is still a well-formed
tool object. FR3a exists for that reason.

## Exchange one, six applied and one referred

**The request was to travel in argv, and that was a real defect.** The
first draft said to pass the request to the child as a single argument.
ssh joins its command arguments into one string and hands it to a shell
on the far side, so a request carrying a quote, a backtick, a dollar
sign, or a semicolon would have been interpreted there rather than
delivered. Request text is corpus-derived and caller-supplied, which
makes argv an injection path rather than a formatting choice. Both
targets now invoke `yeomna call -` and write to stdin (D12), with FR15
requiring byte-identical delivery and EC-9 testing a `query` term of
`"; rm -rf /"` and a body containing `$(id)`. Tested rather than reasoned
about, because the failure is silent on every happy path.

**Protocol conformance, confirmed against the 2026-07-28 text.** Every
result must carry `resultType`, including results carrying
`isError: true`, which is easy to omit because the field belongs to the
envelope MCP wraps rather than to anything Yeomna produces. Both
`protocolVersion` and `clientCapabilities` are required per request while
`clientInfo` is not, malformed metadata is `-32602`, an undeclared
capability is `-32021` with `data.requiredCapabilities`, and an
unsupported version is `-32022`.

**Transport is not a verb.** `yeomna call` returns only 0, 1, or 2, so an
ssh exit of 255 is ssh failing. EC-3 records the honest warrant: 255 is
unambiguous because of what the CLI chooses to return and not because ssh
reserves it.

**Cancellation was thinner than it looked.** D10 absorbed the earlier
thin version and now covers reaping every in-flight child on cancel and
on stdin EOF, and states the limit rather than hiding it. ssh does not
forward signals without a tty, so a remote verb already inside its
transaction may run to completion. The pre-committed audit row is what
makes an abandoned destructive call visible, which is the existing design
rather than a new mechanism.

**Referred rather than fixed: R29.** The review asked either to resolve
it or to build a conditional allowlist first. Resolving it turns on
charter section 6's sentence against a raw query surface set against MCP
tools being model-controlled by design, which is a ruling and goes to
Todd under the workflow. Building the allowlist first costs more than
waiting, because the derivation walks `WIRE_NAMES` and an exclusion is a
filter over the contract rather than a second table. What the PRD does
instead is make the dependency mechanical: a ruling against `sql` changes
D6's count, FR2's census and its success criterion, and nothing else.

**Ruled 2026-09-11, after the merge: tool abstractions only, `sql`
excluded.** The referral was the right call and the ruling came back in one
exchange. Two things were learned in the discussion that the finding had
not raised. The ruling does not rest on blast radius, because R17a had
already closed that: `sql` refuses the appliance's own footing, templates,
and any kg-pattern database by catalog probe, under a role holding no grant
on any KG table, so the corpus was structurally unreachable. It rests on
authorship, which is the axis MCP's model-controlled definition creates and
the CLI does not have. And the exclusion needed two requirements the
finding's "conditional allowlist" framing would have missed: it has to
cover dispatch as well as the list, or the result is a tool nobody
advertises and anybody can call, and the census has to name the absence
rather than count to it, since 41 passes if a different verb went missing.

## Exchange two, four applied, one of them a wrong claim

**D13 was false for a class of verbs.** It had said the graph is always a
request field the contract defines. Checked against the contract:
`QueryRequest` carries no `graph` field at all, `hybrid` reads
`s.graph()` and refuses without one, and the config key for that scope
already exists, documented as "the graph a session starts scoped to when
the caller names none". So both sources are real and which applies is the
contract's business. **This would have failed the acceptance criterion
the spec itself wrote:** a database-only fixture fails with "hybrid
ranking needs a graph" and reads as a defect in the server rather than a
missing key in the fixture. The fixture now names both
`database = "weavertools"` and `graph = "weavertools"`.

Recorded with it, because it is a limit rather than a defect: one server
instance is scoped to one graph for session-scoped verbs, so two graphs
means two entries in the MCP client's config, each with its own
`YEOMNA_CONFIG`.

**The child's stdin was never closed.** `yeomna call -` reads until end
of input, so a child whose stdin stays open waits rather than answering.
Omitting the close hangs the call instead of failing it, which is why
FR18 asserts completion without a timeout: a test that only checked the
result would sit there.

**The caching contract was gestured at rather than stated.** Reading the
caching page properly produced two things beyond the finding.
`tools/call` is not a cacheable operation, so a verb result carries no
hints at all, which is non-goal 9 restated in the protocol's own
vocabulary. And `listChanged` is false, since a list derived from a
compiled-in enum cannot change while the process runs.

**Two stale argv forms.** Exchange one abolished argv in D12 and left the
architecture diagram and the Phase 2 example still showing
`yeomna call <json>`. Both were fixed. One mention remains in the "What
exists" background section, where it accurately describes the merged CLI
and is not a statement about this server.

## The pattern worth keeping

Three of the nine applied findings were defects rather than omissions,
and all three share a shape: **the document asserted a mechanism without
checking it.** The argv form assumed ssh preserves argument boundaries.
D13 assumed a uniform graph source across 42 verbs. The stdin form
assumed a child would answer without being told the input ended. The one
mechanism that was checked before writing, schemars' rendering, is the
one that produced its own finding early and cheaply.

This is the failure mode M5 was written to name, arriving in a document
rather than in a comment. A spec that states a mechanism is making a
claim, and a claim that has not been run is a guess with good formatting.
