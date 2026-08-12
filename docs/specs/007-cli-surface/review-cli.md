# Review Notes: CLI surface capture

Reviewer: Claude, with Todd. Date: 2026-08-12. The final reference read.

## What was captured

The clap definitions were extracted span-for-span from the reference's
`main.rs` and command modules (doc comments, attributes, defaults intact),
assembled into `defs.rs` (about 890 lines), with `output.rs` ported whole as
the envelope-convention capture (marked allow dead_code until the verbs
consume it). `main.rs` dispatches all 56 leaf commands to the hole envelope,
each naming what it awaits. The holes census test runs the real binary over
every command with clap-derived filler arguments and asserts envelope shape
and nonzero exit, so the ledger is machine-checked on every test run.

## Dropped at the door, per standing rulings

`task` and `smell`/`link` (methodology), `db aql` (charter section 6),
`graph-embed train` (training cluster excluded), the daemon `--mcp-*` flags
(MCP parked), `ingest --claims` (CS-NN is smell surface).

## Deviations from a pure capture

- The two `DbSchemaCmd` struct variants needed pattern fixes the generator
  misread, and `NonZeroUsize` needed its import. Mechanical.
- Brand sweep applied at capture (binary `yeomna`, daemon default
  `/run/yeomna/yeomna.sock`, help-text strings), per the naming ruling.
- Q3's ArangoDB-vocabulary command names (`collections`,
  `create-database`, `materialize`, and friends) are captured as-is and
  rename when the verb spec rules, as the spec directs.

## The census, as of capture

56 commands, every one a hole. The awaits distribution: verb layer and
store (28), ingest orchestrator (9), embedder backend (6), graph-embed era
(3), schema manager (1), analyzer toolchain manager (2), daemon (1), and
the six document-level commands split across ingest and extraction
backends. `yeomna --help` renders the kept contract.

## Verification

Workspace gate green (296 tests including the census), clippy clean, fmt
clean. The first brand grep caught four missed lines in the Cli
docs (the about text, the --db doc, a defs comment), swept in a follow-up
commit, after which the grep is clean except reference citations.
