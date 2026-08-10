# PRD: Pipeline Libraries

Status: draft v0.1, 2026-08-10. Sits beneath the Yeomna Charter PRD (README.md,
draft v0.4) and alongside `docs/PRD-postgres-store.md`. Where this document and
the charter disagree, the charter wins.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Revision History

| Version | Date | Change |
|---|---|---|
| 0.1 | 2026-08-10 | First draft. Crate split, dependency order, lift scope. |

---

## Executive Summary

Lift the store-free mass of HADES-Burn into this repository as independently
testable crates, before the Postgres schema exists and without waiting for it.

This is charter section 14's first slice, and it is now also the only body of
work that can proceed on evidence. The reference is not running, so nothing that
depends on observing its behavior can start. These libraries depend on observing
its source, which is available.

About 12.3k lines of Rust and 3.3k lines of Python move with no functional
edits. The measured store coupling in that mass is **zero**, and the two
apparent exceptions dissolve under inspection: `code/` imports `db::keys` and
`db::collections::CODEBASE`, which are key derivation and a struct of static
name strings. Neither is a database.

When this lands, the gap between working tools and an absent store stops being a
design argument and becomes a compile error, which is a better basis for the
schema than an ontology document read in advance.

---

## Background and Context

### The Problem

The store PRD sequences seven phases of schema before the sink at Phase 7. That
ordering assumed the reference could be consulted while the schema took shape.
It cannot.

Verified 2026-08-10: ArangoDB data survives on `dbpool/arangodb` (50.3G
referenced, snapshots to October 2025), ArangoDB the software is not installed,
there is no service unit, HADES-Burn has no `target/` directory, and the
embedder and extractor are down.

### Why Now

Designing tables for output nobody has produced yet is guessing with extra
steps. The analysis and chunking layers emit concrete structures. Once those
structures exist in this repository and are exercised against real source, the
sink's shape is dictated rather than inferred.

### The Opportunity

The port scope measures at about 2 percent of the reference. This PRD covers the
other 98 percent, and it moves nearly intact.

### Development Strategy

PRD, then spec, then build to spec. Migration from the reference is orderly,
reviewed, and documented per file as it lands. A low coupling count makes the
review cheap, not skippable.

---

## User Stories

### Developer

Runs `cargo test` in this repository and sees analysis, chunking, and key
derivation pass against fixtures drawn from real source, with no database of any
kind installed.

### Schema Author

Reads the structs the analysis layer emits and writes tables that accept them,
rather than reading an ontology document and hoping the code agrees with it.

### Reviewer

Reads a diff where each lifted file is accounted for against the spec that
called for it, and can tell what changed from the reference and why.

---

## Goals

### Primary Goals

1. **G1. Store-free by construction.** No crate in this PRD may depend on a
   store crate, because no store crate exists yet to depend on. The property is
   enforced by the dependency graph rather than by discipline.
2. **G2. Independently testable.** Every crate tests without ArangoDB, without
   Postgres, and without the embedder or extractor services running.
3. **G3. Behavior preserved by construction.** The code moves rather than gets
   rewritten, so identical behavior is a property of the change being a move.
   There is no parity percentage to hit and nothing to measure against. The way
   to lose this is to improve the code in transit, which R1 forbids.
4. **G4. No graph vocabulary ahead of a graph.** No primitive, relation, or
   basis enums, and no ontology crate. The lifted code carries its own strings.
   Vocabulary gets defined when the thing that needs it exists.
5. **G5. A sink-shaped hole.** The pipeline orchestrator's four coupling points
   become a trait with no implementation, so the missing piece is visible and
   typed.

### Non-Goals

1. **The sink implementation.** This PRD defines the trait. Implementing it
   against Postgres belongs to the store PRD.
2. **Any schema, table, or SQL.** None of it is needed to lift these libraries
   and writing it here would reintroduce the ordering this PRD exists to fix.
3. **`codebase_ingest.rs`.** Its 3,873 lines carry 67 coupled lines across about
   17 blocks and it is the CLI orchestration of the store write path. It waits
   for the sink.
4. **The CLI, the daemon, dispatch, service, and the verb layer.** Later work.
5. **Porting `db/` beyond `keys.rs` and the naming vocabulary.** The ArangoDB
   client is left behind entire.
6. **Resurrecting the reference.** Named as an option in the store PRD's testing
   section, not undertaken here.
7. **Rewriting or improving the lifted code.** A port is not a refactor. Defects
   found are recorded, not fixed in the same change.

---

## Technical Architecture

### The measured dependency graph

Taken from the reference on 2026-08-10, not assumed:

| Module | LOC | `crate::` deps | External crates |
|---|---|---|---|
| `chunking/` | 733 | **none** | **none, pure std** |
| `batch/` | 1,220 | **none** | serde, serde_json, tokio, tracing |
| `persephone/` | 1,067 | **none** | hades_proto, hyper, hyperlocal, tonic, tower, http |
| `db/keys.rs` | 493 | **none** | regex, sha2 |
| `code/` | 9,294 | `db::keys`, `db::collections::CODEBASE`, `chunking` | clang, syn, rustpython_parser, tree_sitter, regex, sha2, url |
| `pipeline/` | 647 | `db::ArangoPool`, `db::ArangoError` | tokio, tracing |

An apparent `code/ -> config` dependency was a false positive. The matches are
string literals inside import-resolution test fixtures, such as
`vec!["crate::config::Config".to_string()]`. There is no real dependency.

### Crate layout

```text
crates/
  yeomna-chunking/   none             <- chunking/                     733
  yeomna-keys/       regex, sha2      <- db/keys.rs                    493
  yeomna-batch/      serde, tokio     <- batch/                      1,220
  yeomna-proto/      tonic, prost     <- hades-proto                    28 + .proto
  yeomna-embed/      -> proto         <- persephone/                 1,067
  yeomna-code/       -> keys,         <- code/                       9,294
                        chunking
  yeomna-pipeline/   -> all above     <- pipeline/ minus the store      647
services/            unchanged        <- Python                      3,320
```

One crate is an addition forced by the measurement: **`yeomna-proto`** exists
because `persephone/` imports `hades_proto`. It is 28 lines of `lib.rs` over
generated protobuf, plus a `build.rs`.

**There is no vocabulary or ontology crate, by ruling (2026-08-10): no graph
vocabulary ahead of a graph.** The `db::collections::CODEBASE` struct that
`code/` imports is eight static name strings, and the only lifted consumers are
inside `code/` itself, so the strings land as an internal module of
`yeomna-code` during Phase 4, marked transitional. The lang-kind-to-primitive
mapping already lives in `code/symbols.rs` and `code/lsp/edges.rs` as code and
moves with the lift. Enums for primitives, relations, or basis get defined when
the thing that needs them exists, which for basis is the store schema and for
everything else is post-hoc dogfooding.

### The sink-shaped hole

`pipeline/orchestrator.rs` couples at four points: the `ArangoPool` import, an
error variant, the `db` struct field, and the constructor parameter. Those
become a trait with no implementor in this PRD.

```rust
// yeomna-pipeline: the shape of what is missing.
pub trait IngestSink {
    type Error: std::error::Error + Send + Sync + 'static;
    // Method set is deliberately unfixed at v0.1. It gets settled by what the
    // analysis layer actually emits once yeomna-code compiles here, which is
    // the point of doing this before the schema.
}
```

Leaving the method set open is deliberate. Fixing it now would be the same
mistake as writing the tables now.

---

## Feature Specifications

Phases are ordered by the dependency graph. Each is a candidate spec.

### Phase 1: Workspace and chunking

Create the Cargo workspace, edition 2024, and land the first lift:
`yeomna-chunking`, which has no internal dependencies and no external crates.
The cheapest possible calibration of what "reviewed and documented as it lands"
costs per file. Specced at `docs/specs/001-workspace-and-chunking/spec.md`.

### Phase 2: The remaining leaves

`yeomna-keys` and `yeomna-batch`. No internal dependencies, so both can move in
parallel.

Key derivation carries a hard requirement: **determinism must be preserved
exactly**. `symbol_key`, `file_key`, `chunk_key`, and `edge_key` produce the
values that make re-ingest idempotent. Their outputs are a contract, and the
tests must pin known inputs to known outputs rather than merely check that the
functions run.

### Phase 3: The embedder path

`yeomna-proto` then `yeomna-embed`. The contract is PE-API v1, documented at
`docs/persephone-embedding-api.md` in the reference. These clients talk to
services that are currently down, so tests must not require a live embedder.

### Phase 4: Analysis

`yeomna-code`, 9,294 lines, the largest single lift. Depends on Phases 1 and 2.
Its `db::keys` imports are rewritten to `yeomna_keys`, and its
`db::collections::CODEBASE` imports point at an internal `containers` module of
this crate holding the same eight static name strings, marked transitional in
its docs since the sink strips the prefix they encode. Those import lines plus
that one small module are the only edits the port requires, and after them the
crate has no store dependency of any kind.

The seven ArangoDB references remaining in `code/` are doc comments such as
`/// ArangoDB document key`. They are corrected as documentation, and the
correction is part of the review rather than a follow-up.

Analysis is tiered: `semantic` (rust-analyzer, gopls, libclang), `structural`
(syn, tree-sitter, Python AST), and `text`. Tiers are ordered and cannot
silently downgrade on re-ingest. libclang is dlopened at runtime and degrades
gracefully when absent, which the lift must preserve.

### Phase 5: Orchestration, minus the store

`yeomna-pipeline`, 647 lines, with the four coupling points replaced by the
trait. This is the phase that makes the hole visible and typed.

### Phase 6: Python services

`services/` moves unchanged: extraction (Docling, LaTeX backend, PyMuPDF
fallback) and embedding (Jina V4). Zero references to any store across 32 files.
These are separate processes behind a socket and an HTTP contract and they are
repointed rather than ported.

Deferred deliberately: the extraction service pins `cuda:2` with a 15-minute
idle VLM unload, and the embedder wants its own GPU. Charter 5.1 already
requires local embedding, so this is that requirement's hardware bill, not a new
problem. Standing these services up is not required to lift them.

---

## Testing Strategy

**No database, no services.** G2 is the test of whether the lift is honest. If a
test needs ArangoDB, Postgres, or a live embedder, either the test is wrong or
the crate boundary is.

**Golden-value tests on key derivation.** Determinism is a contract. Pin
specific inputs to specific outputs, taken from the reference's documented
examples such as `symbol_key("src_lib_rs", "Config::new")` producing
`src_lib_rs__Config__new__<hash8>`.

**Self-application as the corpus.** Run analysis and chunking over this
repository and over HADES-Burn's own tree. Both are on disk, both are real, and
neither needs a store. This is dogfooding arriving early and cheaply, on the
tools rather than on the graph.

**Ported tests come with the code.** The reference has 2,423 lines of tests
across 15 files. The ones covering lifted modules move in the same change as the
code they cover.

**No comparison against the reference.** G3 is preserved by the change being a
move, not by a harness. Read the diff, confirm it is a relocation plus rewritten
import lines, and let the ported tests do the rest. Nothing here benchmarks or
diffs the two systems, because the reference was displaced over a license and
resembling it more closely buys nothing.

---

## Risk Assessment

### R1. The lift becomes a refactor.

9,294 lines of analysis code invite improvement while passing through. G3 holds
because the change is a move, and every in-transit edit is a hole in that
argument and a line a reviewer has to reason about instead of skim.

Amended 2026-08-10 by the per-crate PR workflow: defects and review findings
are fixed on the lift PR as separate commits after the verbatim move commit,
rather than deferred to a backlog. The move commit itself stays untouched, as
the only surviving record of the reference's behavior. Fix freely above it,
never inside it.

### R2. Vocabulary sneaks back in ahead of the schema.

The pull is constant: a primitive enum here, a basis type there, each one
reasonable alone. Every one of them designs the graph before the graph exists
and duplicates strings the lifted code already carries. G4 is the rule, and the
store PRD owns the schema when its time comes.

### R3. The sink trait gets designed too early.

Fixing the method set before `yeomna-code` compiles here reproduces the ordering
mistake this PRD corrects. Leave it open until Phase 5 has real callers.

### R4. Determinism drifts silently.

A changed hash input produces different keys, re-ingest stops being idempotent,
and nothing fails loudly. Golden-value tests are the control and they are not
optional.

### R5. libclang and language servers are environment-dependent.

The reference dlopens libclang and degrades to a lower tier when it is absent.
Tests must cover the degraded path, since CI and a developer box will differ.

---

## Timeline

Ordered by dependency, no dates.

| Phase | Depends on | Unblocks |
|---|---|---|
| 1. Workspace and chunking | Nothing | 2, 3, 4 |
| 2. keys, batch | 1 | 4, 5 |
| 3. proto, embed | 1 | 5 |
| 4. code | 1, 2 | 5 |
| 5. pipeline and the sink trait | 2, 3, 4 | Store PRD Phase 7 |
| 6. Python services | Nothing | Ingest end to end |

Phase 5 is the handoff. When the trait exists with no implementor, the store PRD
stops being speculative.

---

## Appendix A. Source Map

| Target crate | Reference path | LOC |
|---|---|---|
| `yeomna-keys` | `crates/hades-core/src/db/keys.rs` | 493 |
| `yeomna-chunking` | `crates/hades-core/src/chunking/` | 733 |
| `yeomna-batch` | `crates/hades-core/src/batch/` | 1,220 |
| `yeomna-proto` | `crates/hades-proto/` | 28 plus generated |
| `yeomna-embed` | `crates/hades-core/src/persephone/` | 1,067 |
| `yeomna-code` | `crates/hades-core/src/code/` | 9,294 |
| `yeomna-pipeline` | `crates/hades-core/src/pipeline/` | 647 |
| `services/` | `services/` | 3,320 Python |

## Appendix B. What This Changes in the Store PRD

The store PRD's Phase 7 (the sink) now consumes a trait defined here rather than
inventing one. Its Testing Strategy has been rewritten to drop the parity
percentage and the differential harness. Its phase ordering is unchanged, but it
is no longer the only work that can proceed.
