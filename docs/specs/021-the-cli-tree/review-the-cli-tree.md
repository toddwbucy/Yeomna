# Review notes: 021 the contract-born CLI tree, and H8

Status: build complete 2026-09-10, local review run before the PR
opened. **R24 proposed** on what `tools install` may reach for.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## What landed

The per-verb tree, the human rendering, `--json`, and H8's two commands.
All seven of H2's phases are now done, which finishes the verb layer,
and epic #37 is complete.

**The tree has no separate existence.** Every subcommand path is a wire
name with its dots as spaces (D10), the flags become a JSON object, and
the request is built by deserializing into the closed enum. No verb has
hand-written argument code. Epic #37 asked for a tree exhaustively
matched over `Verb` so a new verb fails compile until the CLI carries
it, and this is stronger: the tree is derived, so a verb added to the
contract is reachable the same day and cannot be forgotten.

Arguments arrive typed by parsing each value as JSON and falling back to
a string, so `--depth 5` is a number, `--force` is `true`, and
`--relations '["calls"]'` is an array. Every check is the contract's: an
unknown field, a missing required field, and a wrong type are all
deserialization errors naming what was wrong.

## The collision dogfooding found

`--graph` is both the session-scoping flag and a request field on many
verbs. The first cut consumed it as the session flag, so
`yeomna graph neighbors --graph yeomna_self ...` lost it and failed on a
missing field. Found by running the binary rather than by a test, which
is why the dogfood step is in the workflow.

The fix keeps one meaning for a person: `--graph` is offered to the verb
*and* scopes the session, and when the contract answers "unknown field
`graph`" the flag is dropped from the args and the session keeps it. The
contract decides, so no list of graph-carrying verbs is maintained here.
Both paths have a test.

## The five things the capture got wrong, and where each is answered

1. **No `--mcp-*` flags.** None exist. MCP is parked.
2. **Headers with rows.** `render.rs` builds header, rule, and rows into
   one string on stdout, with a unit test pinning the exact three lines
   and an integration test asserting stderr is empty.
3. **Counts from the contract.** The census test asserts
   `WIRE_NAMES.len() == 42` rather than keeping a number here.
4. **The R4 reconciliation.** Already done: spec 010's table is the only
   surface list.
5. **Verbs with no captured ancestor.** `edge.assert` and `edge.retract`
   reach the tree like every other name, because the tree is derived.

## The census, inverted

PR #19's census asserted that every command reports its hole. This
asserts that every wire name is reachable as a subcommand. Reachability
is tested by whether the path resolves rather than by whether the verb
succeeds: a resolved path fails on a missing field or an absent store,
and only an unresolved one says "unknown command". So the test runs
without a cluster and still proves the tree covers the contract.

## R24, proposed

`tools install` places a binary the operator already has and does not
fetch from upstream. Fetching means deciding what the appliance may talk
to, how a release is verified, and whether an operator's convenience
outranks the seal, and those are charter section 5 questions rather than
an implementation detail. The command says so when asked to install
without a source, and the message names the ruling.

## Build findings

**A test mutated the environment, again.** The first `install` test set
`YEOMNA_TOOLS_DIR` in process to point the resolver at a temporary
directory, which is the process-wide mutation this session has now
flagged three times. Split into `install` and `install_in` the way
`resolve_and_probe` and `resolve_and_probe_in` already are, and the test
calls the explicit form.

**H8 was the shallowest hole and it stayed shallow.** `resolve_and_probe`
did the work already, so `status` is a table over it. Reusing it rather
than probing separately is the point: the preflight and the report
cannot drift on resolution order or on how a version is asked for.

## Test inventory

32 in `yeomna-cli` (14 unit, 18 through the binary). New: the census
inversion, typed arguments with three contract refusals, the near-miss
suggestion, table-versus-JSON with exit codes, the header-with-rows
assertion, H8's four cases including R24's refusal, and the `--graph`
collision from both sides.

## Riding items

- R24, above.
- Rendering has no width limit, so a very wide table wraps in a narrow
  terminal. A `--wide` or a truncating column is presentation work for
  when someone minds.
- `yeomna call` remains the agent surface and always prints the
  envelope. The tree is the person's, and the split is deliberate.
