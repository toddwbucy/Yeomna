# Specification: 007 CLI Surface Capture

Parent PRD: `docs/PRD-pipeline-libraries.md`, added by the v0.2 severance
ruling.
Status: draft, 2026-08-12.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

The final reference read. `yeomna-cli` captures the reference CLI's
**surface**: the clap command tree, argument structures, help text, and the
`output.rs` JSON-envelope conventions that AGENTS.md documents. Every handler
is a uniform self-reporting hole. The result is the executable holes ledger:
the CLI compiles, runs, `--help` renders the kept contract, and every command
answers with the name of what it awaits.

This is capture plus construction, like 005: the definitions move, the
handlers are new stubs, and no dispatch, config, or store code comes along.

## The hole contract

Every stubbed command emits one envelope shape to stdout and exits nonzero:

```json
{"success": false, "hole": true, "error": "hole: db query awaits the verb layer and the store"}
```

Each hole names its dependency, so the ledger reads from the tool itself:
`awaits the store`, `awaits the verb layer`, `awaits the ingest
orchestrator`, `awaits the embedder backend`, `awaits the daemon`, `awaits
the schema manager`. `yeomna --help` plus a loop over subcommands is the
holes census.

## Captured, dropped, and renamed

**Captured** (definitions and help text verbatim except the naming sweep):
`status`, `orient`, `extract`, `ingest`, the `db` tree (query, list, stats,
recent, health, check, purge, create, delete, collections, databases,
create-database, plus the read, write, search, graph, and schema subtrees),
`embed` (text, service mgmt), `codebase` (ingest, update, prune-orphans,
drift, validate, retire), `graph-embed` (embed, neighbors, update), `schema`
(apply), `tools` (status, install), `daemon`. Global `--db` and `--gpu`
args. `output.rs` whole, it is store-free.

**Dropped at the door, per standing rulings:** `task` (methodology), `smell`
and `link` (methodology), `db aql` (charter section 6, stays dead),
`graph-embed train` (training cluster excluded), the daemon's `--mcp-*`
flags (MCP parked), and `ingest --claims` (CS-NN compliance flags are smell
surface).

**Renamed:** binary `yeomna`, every hades string in help text and defaults
(`/run/yeomna/yeomna.sock` for the daemon default), Q3's ArangoDB-vocabulary
verb names captured as-is and renamed later when the verb spec rules,
since renames inside this repo are cheap and the capture should be
mechanical.

## Requirements

**FR-CLI1.** `cargo run -p yeomna-cli -- --help` renders the kept tree, and
every leaf command runs, emits the hole envelope, and exits nonzero.

**FR-CLI2.** `output.rs` conventions preserved: JSON to stdout, logs and
diagnostics to stderr, format flags accepted (holes render as JSON regardless
of format, stated in help).

**FR-CLI3.** No dependency on any store, service client, dispatch, or config
layer. The crate depends on clap, serde_json, anyhow, chrono, tracing at
most.

**FR-CLI4.** A `holes` test walks every command and asserts the envelope
shape and exit behavior, so the census is machine-checked.

**FR-CLI5.** Arg structures and doc comments match the reference for kept
commands, verified by review reading rather than diff, since extraction from
handler-bearing files is not a file move.

## Out of Scope

- Implementing any command. Every handler is a hole until its era.
- The config layer, `--db` resolution semantics, and the
  no-compiled-in-default warning from AGENTS.md, which arrive with the verb
  layer.
- Q3 renames, ruled at the verb spec.

## Success Criteria

1. Build, test, clippy, fmt clean, workspace-wide.
2. The holes test passes and the census lists every kept command.
3. Store-reference and brand greps clean except provenance.
4. Review notes record every captured module, every dropped item, and every
   deviation from the reference's argument shapes if any prove necessary.
