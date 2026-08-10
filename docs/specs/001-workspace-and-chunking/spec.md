# Specification: 001 Workspace and Chunking

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 1.
Status: draft, 2026-08-10.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

Create the Cargo workspace for Yeomna and land the first lift:
`yeomna-chunking`, 733 lines from the reference's `chunking/` module. It has no
internal dependencies, no external crates, and no store coupling, which makes it
the cheapest possible calibration of what "reviewed and documented as it lands"
costs per file.

A standing ruling, recorded here because this spec replaced one that broke it:
**no graph vocabulary ahead of a graph.** No primitive enums, no relation enums,
no basis types, no ontology crate. The lifted code already carries the strings
it writes, and the store schema will be shaped by the emitted types when the
lift is done. Graphing of this repository's own code and documents is post-hoc
work that arrives with dogfooding, not scaffolding to build now.

## Workflow Type

Workspace scaffolding (new, small) plus a lift (move, no functional edits).

## Task Scope

### This Task Will

1. Create a workspace `Cargo.toml` at the repository root, edition 2024.
2. Create `rust-toolchain.toml` pinning the channel.
3. Move `chunking/` from the reference into `crates/yeomna-chunking`, including
   its tests.
4. Leave `cargo build`, `cargo test`, `cargo clippy`, and `cargo fmt --check`
   passing from the repository root.

### Out of Scope

- **Any vocabulary, ontology, or basis type.** Per the ruling above. The
  container name strings that `code/` imports arrive in Phase 4 as an internal
  module of `yeomna-code`, because the only lifted consumers are inside `code/`
  itself.
- **Any other crate.** `yeomna-keys`, `yeomna-batch`, `yeomna-proto`,
  `yeomna-embed`, `yeomna-code`, `yeomna-pipeline` are later phases. Do not
  list unbuilt crates as workspace members.
- **Any table, column, SQL, or store trait.**
- **Improvements to the chunking code.** Defects found are recorded in the
  review notes, not fixed in transit, per PRD risk R1.

## Repository Context

There is no code in this repository today. `.gitignore` is Cargo's, so the
layout below is what it was already written for.

Rust is available through rustup with `stable` 1.97.1 installed, but **no
global default toolchain is configured**, so a bare `cargo` command fails
today. `rust-toolchain.toml` in this task fixes that for this repository
without touching the global setting.

## Files to Create

| Path | Source |
|---|---|
| `Cargo.toml` | New. Workspace root. |
| `rust-toolchain.toml` | New. Matches the reference's shape. |
| `crates/yeomna-chunking/Cargo.toml` | New. No dependencies. |
| `crates/yeomna-chunking/src/lib.rs` | From `chunking/mod.rs` |
| `crates/yeomna-chunking/src/strategies.rs` | From `chunking/strategies.rs` |
| `crates/yeomna-chunking/src/late.rs` | From `chunking/late.rs` |

## Files to Reference

| What | Where |
|---|---|
| The module being lifted | `~/olympus/HADES-Burn/crates/hades-core/src/chunking/` |
| Workspace conventions to match | `~/olympus/HADES-Burn/Cargo.toml`, `rust-toolchain.toml` |
| The lift rules | `docs/PRD-pipeline-libraries.md`, G3 and R1 |

## Patterns to Follow

### Workspace manifest

Match the reference's shape: edition 2024, resolver 2, shared metadata in
`[workspace.package]`. Crate metadata uses the string `yeomna`, per charter
section 13.

```toml
[workspace]
members = ["crates/yeomna-chunking"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"

[workspace.dependencies]
# Populated as crates land. This phase needs nothing.
```

### The lift is a move

`mod.rs` becomes `lib.rs` and inner `mod` declarations adjust to the crate
root. Those wiring edits, plus crate-level docs, are the entire allowed diff.
The review reads the diff against the reference to confirm exactly that.

## Requirements

### Functional Requirements

**FR1.** `yeomna-chunking` exports the same public API the reference's
`chunking` module exports: `TextChunk`, the strategy types (`TokenChunking`,
`SlidingWindowChunking`, `SentenceChunking`), the `ChunkingStrategy` trait, and
the late-chunking items, under the same names.

**FR2.** The crate has zero dependencies. The reference module uses only std.

**FR3.** All `#[cfg(test)]` tests from the reference module move with their
files and pass unchanged.

**FR4.** Chunk output is byte-identical to the reference for the same input
and strategy configuration. The tests moved in FR3 are the evidence, since
they pin offsets and indices.

### Edge Cases

**EC1.** Empty input produces whatever the reference produces for empty input.
Do not "fix" it either way. If it looks wrong, record it.

**EC2.** Multi-byte UTF-8 near chunk boundaries: the reference's behavior is
the specification. The moved tests cover what they cover, and no new boundary
policy is invented here.

## Implementation Notes

### DO

- Read every file before moving it. The review happens as the code lands, not
  after.
- Keep a per-file note in the review: source path, line count, what changed
  (expected answer: module wiring only), anything odd worth recording.
- Run `cargo clippy` and leave it clean. If clippy flags reference code,
  silence with a scoped `#[allow]` and a comment naming this spec, rather than
  editing the logic.

### DON'T

- **Do not refactor, rename, or improve.** R1.
- **Do not add serde or any other dependency.**
- **Do not define any graph vocabulary.** The ruling at the top of this spec.
- **Do not list unbuilt crates as workspace members.**

## Development Environment

```bash
cd /home/todd/git/Yeomna
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

Nothing in this task needs Postgres. The development cluster can be stopped and
every test still passes, which is the point.

## Success Criteria

1. `cargo build`, `cargo test`, `cargo clippy --all-targets`, and
   `cargo fmt --check` all pass from the repository root.
2. `crates/yeomna-chunking/Cargo.toml` has an empty `[dependencies]` section.
3. `diff -r` between the crate's `src/` and the reference's `chunking/` shows
   only module wiring and crate docs.
4. No file in the crate contains the string `arango`, `sql`, or `postgres`,
   case-insensitive. (`Pool` was in this list as drafted and was struck during
   execution: it matched the crate's own mean-pooling vocabulary, and the
   store type it aimed at, `ArangoPool`, is already caught by `arango`.)
5. Every test passes with the Yeomna Postgres cluster stopped.

## QA Acceptance Criteria

1. The reference's chunking tests are present and passing, unchanged.
2. The review notes exist and account for every moved file.
3. The workspace builds with exactly one member.
