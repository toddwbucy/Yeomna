# Specification: 021 The Contract-Born CLI Tree, and H8

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 7, the last phase.
Owner: epic #37's third and fourth parts, H8. Ruled by R21 D10 (wire
dots become spaces) and R20 (the tree is born from the contract, never
from the retired capture). The eighth PR in R21's order.
Status: draft, 2026-09-10. **R24 proposed** on what `tools install` may
reach for.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

`yeomna call` serves an agent, which wants JSON in and an envelope out.
A person wants to type `yeomna graph neighbors --key foo --direction in`
and read a table. This spec adds that surface, and H8's two toolchain
commands with it, which closes the last phase of the verb layer and
epic #37.

The shape is the one R20 argued for when the captured CLI closed
unmerged: **the tree is derived from the contract rather than written
beside it.** Every subcommand path is a wire name with its dots turned
into spaces (D10), every request is built by deserializing into the
closed `Verb` enum, and no verb has hand-written argument code. A verb
added to the contract therefore gets a subcommand the same day, which
is stronger than the compile-time completeness epic #37 asked for: the
tree cannot lag the contract because it has no separate existence.

That also retires the capture's census by inversion, as planned. PR #19
asserted that every command reports its hole. The test here asserts that
every wire name is reachable as a subcommand and reaches its verb.

## The five things the capture got wrong, and are not repeated

Carried from PR #19's review and recorded in epic #37:

1. No `--mcp-*` flags. MCP is parked, the capture declared flags its own
   record said were dropped, and one of them named a file that never
   existed.
2. Headers and rows go to the same stream. The capture printed table
   headers to stderr and rows to stdout, so redirecting stdout lost the
   header.
3. Counts come from the contract. The capture's own numbers reached 56
   by double-counting six commands.
4. The R4 reconciliation is already done: spec 010's disposition table is
   the only surface list, and it says 42.
5. Two verbs have no captured ancestor at all (`edge.assert` and
   `edge.retract`, added by R18), which is the plainest argument for
   deriving the tree from the contract rather than from what a
   predecessor happened to expose.

## Task Scope

- The per-verb tree in `yeomna-cli`: `yeomna <path...> [--key value]`,
  where the path is a wire name with dots as spaces.
- Generic argument parsing: `--key value` pairs become a JSON object,
  each value parsed as JSON first so numbers, booleans, and arrays
  arrive typed, falling back to a string when that fails. A bare
  `--key` with no value is `true`. The object becomes
  `{"verb": name, "args": {...}}` and deserializes into `Verb`, so the
  contract does every check including its refusal of unknown fields.
- Human rendering by default for the tree, with `--json` for the raw
  envelope. `yeomna call` stays raw always, because its caller is a
  program.
- `yeomna tools status`: resolve and probe each analyzer through
  `yeomna_code::lsp::resolve_and_probe`, the same call the ingest
  preflight makes, so the two cannot drift on resolution order.
- `yeomna tools install --from <path>`: place a binary the operator
  already has into the managed tools directory, mode 0755. **R24**:
  fetching from upstream is not in scope, because a byte arriving from
  the network into an appliance is a charter-section-5 question and not
  a convenience. The command says so when asked to install without a
  source.
- The census inversion test.

## Out of Scope

- Fetching tools over the network (R24, above).
- Any second way to say what a verb is. The enum stays the grammar.
- Shell completion, config editing, and interactive prompts.
- Anything that reaches the store without a verb.

## Files to Modify

- `crates/yeomna-cli/src/main.rs`: the tree, the dispatch, `--json`.
- `crates/yeomna-cli/src/render.rs` (new): envelope to text.
- `crates/yeomna-cli/src/tools.rs` (new): H8.
- `crates/yeomna-cli/Cargo.toml`: `yeomna-code`.
- `crates/yeomna-cli/tests/cli.rs`: the tree tests and the census.
- `docs/holes.md`, `CLAUDE.md`, epic #37.

## Files to Reference

- `crates/yeomna-verbs/src/verb.rs`: `WIRE_NAMES` and the request
  structs, which are the tree and the argument names.
- `crates/yeomna-code/src/lsp/session.rs`: `resolve_and_probe`,
  `managed_tools_dir`, `AnalyzerStatus`.

## Functional Requirements

- **FR1** `yeomna status` and `yeomna graph list` run their verbs.
  Every wire name is reachable with its dots as spaces, proven by a test
  that walks `WIRE_NAMES`.
- **FR2** Arguments arrive typed: `--depth 5` is a number, `--force` is
  `true`, `--relations '["calls"]'` is an array, `--graph g` is a
  string.
- **FR3** A misspelled argument is a usage error naming it, because the
  contract denies unknown fields. A missing required argument is a usage
  error naming it, because serde says which.
- **FR4** The default rendering is human-readable and goes entirely to
  stdout, headers included. `--json` prints the envelope verbatim.
- **FR5** Exit codes match `yeomna call`: 0 the verb answered, 1 it
  refused or failed, 2 the caller or the machine was wrong.
- **FR6** `yeomna tools status` reports each analyzer's resolved
  command, where it came from, whether it was pinned, and its version or
  its probe error.
- **FR7** `yeomna tools install --from <path>` copies the binary into
  the managed directory and makes it executable, and refuses without
  `--from` naming R24.
- **FR8** The census inverts: a test asserts every wire name in the
  contract is reachable as a subcommand, and none is missing.

## Edge Cases

- **EC-1** An unknown subcommand path: usage error listing the closest
  matches rather than the whole tree.
- **EC-2** `--key` at the end with no value: `true`, which is what a
  flag means.
- **EC-3** A value that looks like JSON but is meant as a string, for
  example a key named `123`: `--key 123` types it as a number, and the
  contract refuses it if the field is a string, so the error names the
  field. Documented rather than worked around, since a caller who needs
  a literal can use `yeomna call`.
- **EC-4** `tools install --from` naming a path that is not a file:
  usage error.
- **EC-5** `tools status` where an analyzer is absent: reported with its
  probe error, exit 0, because reporting absence is the command's job.
- **EC-6** A verb that refuses (H4, H7, H9): the refusal renders and the
  exit code is 1, the same as through `call`.

## Implementation Notes

DO:

- DO build the tree from `WIRE_NAMES` at runtime. A hand-written table
  would be a second contract.
- DO put headers and rows on stdout together.
- DO reuse `resolve_and_probe` rather than probing differently.
- DO run the gate three times before calling it done.

DON'T:

- DON'T hand-write per-verb argument parsing.
- DON'T add `--mcp-*` anything.
- DON'T fetch over the network (R24).
- DON'T emit SQL.

## Success Criteria

1. Every wire name is reachable as a subcommand and reaches its verb.
2. H8 is filled: status probes and install places.
3. Epic #37 is complete and H2's Phase 7 is done, which finishes the
   verb layer.
4. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR8 and EC-1 through EC-6.
- The no-SQL lint still passes with `yeomna-cli` outside its allowlist.
