# Specification: 004 Code Analysis

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 4.
Status: draft, 2026-08-11.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

Lift the analysis engine: `yeomna-code`, 9,294 lines across 21 files, the
largest single move of the port. Four language paths (Rust via syn plus
rust-analyzer, Python via rustpython, C/C++/CUDA via libclang, Go via gopls,
with tree-sitter as the structural fallback), AST-aware chunking over
`yeomna-chunking`, symbol and edge extraction keyed by `yeomna-keys`, and the
five-primitive taxonomy carried as code.

This crate is why the schema can be written from emitted types: `FileAnalysis`,
`Symbol`, `SymbolDocument`, `CrateEdge`, and `TextChunk` are the structures the
sink will receive, and after this lift they compile in this repository.

124 unit tests move with the code, plus three analyzer probes that skip rather
than fail when their tool is absent.

**Standing note, ruled 2026-08-11: this crate is a candidate independent
crate**, on the level of `limen-tree` and `drey`. A multi-lingual code-graph
extractor with no store, no service, and no Yeomna coupling beyond two small
sibling crates is useful far outside this appliance, WeaverTools tooling
included. Nothing in this lift changes for that, and the candidacy is a
reason to keep the crate's boundary as clean as the lift leaves it.

## Workflow Type

One lift, one branch (`lift/004-code`), one Issue, one draft PR. The move
commit is large and its review is made tractable by the two-layer structure:
the mechanical check is `diff -r` against the reference showing only the named
edit classes, and human attention goes to the deltas and the tightening.

## Task Scope

### This Task Will

1. Create `crates/yeomna-code` from `crates/hades-core/src/code/`, preserving
   the module tree including `lsp/`.
2. Add the internal `containers` module: the eight transitional name strings
   `code/` imports from the reference's `db::collections::CODEBASE`, per the
   PRD ruling. Struct and statics only, none of the 308-line profile machinery.
3. Rewrite three import classes and one doctest-path class, listed under
   Allowed Edits.
4. Move the three analyzer probe tests from the reference's
   `crates/hades-core/tests/`: `ra_span_agreement.rs`, `gopls_semantic.rs`,
   `clang_cuda_probe.rs`. All three skip when their analyzer is unavailable,
   and that semantic is part of what moves.
5. Correct the seven ArangoDB doc-comment mentions and the small HADES
   branding set in a tightening commit, per the PRD's Phase 4 text.
6. Leave build, test, clippy, fmt clean at merge.

### Out of Scope

- **`pipeline/` and the sink trait.** Phase 5.
- **Any store crate, table, or SQL.**
- **Primitive, relation, or basis enums.** The lang-kind-to-primitive mapping
  lives in `symbols.rs` and `lsp/edges.rs` as string matches and moves as
  code, per G4. Extracting vocabulary remains forbidden.
- **Analyzer upgrades or new language support.** The lift moves what exists.
- **The 003 follow-up, recorded here so it is not lost:** the reference's
  `embedding_client.rs` and `extraction_client.rs` integration tests were not
  moved with `yeomna-embed`. Their type and config tests run without a
  service and belong in that crate. A small separate PR, not this one.

## Files to Create

Every `code/` file moves 1:1 into `crates/yeomna-code/src/`, with `mod.rs`
becoming `lib.rs` (239 lines). The full inventory, by size: `python.rs`
(1,186), `rust_ast.rs` (1,000), `cpp.rs` (789), `lsp/edges.rs` (769),
`lsp/session.rs` (649), `rust_imports.rs` (592), `lsp/rust_symbols.rs` (542),
`python_calls.rs` (488), `symbols.rs` (461), `tree_sitter.rs` (443),
`lsp/client.rs` (362), `lsp/go_symbols.rs` (348), `cpp_edges.rs` (309),
`chunking.rs` (291), `lsp/rust_analyzer.rs` (265), `language.rs` (165),
`tree_sitter_edges.rs` (153), `lsp/symbols.rs` (106), `lsp/gopls.rs` (91),
`lsp/mod.rs` (46).

New files: `src/containers.rs` (transitional name strings, documented as
such), `Cargo.toml`, and `tests/` holding the three moved probes.

## Allowed Edits in the Move Commit

1. `mod.rs` to `lib.rs` and module wiring.
2. `use crate::db::keys` and `keys::` paths to `yeomna_keys`.
3. `use crate::db::collections::CODEBASE` to `crate::containers::CODEBASE`.
4. `use crate::chunking::...` to `yeomna_chunking::...`.
5. Doctest crate paths, `hades_core::code::...` to `yeomna_code::...`, the
   same class the keys lift established.
6. The provenance note in crate docs, and the new `containers.rs`.

Nothing else. The `#[serde(rename = "_key")]` attributes and every emitted
field name are sink wire shape and move untouched. ArangoDB and HADES doc
mentions survive the move commit and die in tightening.

## Dependencies, measured by reading

The PRD table for `code/` was incomplete, the fifth correction in this
pattern, and the compiler remains the final authority at move time:

| Kind | Crates |
|---|---|
| Internal | `yeomna-keys`, `yeomna-chunking` |
| External | clang, proc-macro2, regex, rustpython-parser, serde, serde_json, sha2, syn, thiserror, tokio, tracing, tree-sitter, tree-sitter-cpp, tree-sitter-go, tree-sitter-python, tree-sitter-rust, url |

Version pins copy the reference exactly, and two matter beyond convention:

- `clang = { version = "2.0", features = ["clang_10_0", "runtime"] }`. The
  `runtime` feature dlopens libclang so the build does not hard-require
  libclang-dev and the analyzer degrades gracefully when it is absent.
- Grammar pins: tree-sitter 0.26.11, cpp 0.23.4, go 0.25.0, python 0.25.0,
  rust 0.24.2. A grammar bump changes parses, which changes symbols, which
  changes derived keys. Grammar versions are behavior.

## Requirements

**FR-C1.** Public API unchanged: `FileAnalysis`, `Symbol`, `SymbolKind`,
`AnalysisTier`, `Language`, `CodeMetrics`, `TopLevelDef`, the analyzer entry
points, and the `lsp` module surface (`CrateEdge`, `SymbolDocument`,
`EdgeKind`, the sessions), under the same names.

**FR-C2.** `containers.rs` holds the `CodebaseCollections` struct and the
`CODEBASE` static with its eight names, verbatim values, documented as
transitional: the sink strips the prefix these names encode, and the module
must not grow a registry, a profile, or a builder.

**FR-C3.** Analysis tiers stay ordered (`text < structural < semantic`) and
the no-silent-downgrade-on-reingest semantic moves intact.

**FR-C4.** Degradation paths are preserved and stay tested: libclang absent
means tree-sitter or text tier with `fallback_reason` recorded, rust-analyzer
and gopls absent mean the syn and AST passes stand alone, and the three probe
tests skip rather than fail. No test in this crate may require an analyzer,
a service, or a GPU to pass.

**FR-C5.** The two-pass enrichment protocol moves intact: syn writes first,
the language server overwrites the same key, and key identity rides the
line-number agreement that `ra_span_agreement.rs` guards. That test is the
cross-crate contract check between `yeomna-code` and `yeomna-keys`.

**FR-C6.** The lang-kind-to-primitive mapping stays string matches in code.
No enum extraction, per G4.

**FR-C7.** After the import rewrites the crate has no store dependency of any
kind, held by the dependency graph and checked by grep at merge.

### Edge Cases

**EC-1.** A file whose preferred analyzer fails mid-parse falls back a tier
with `fallback_reason` populated, and the moved tests that pin this keep
pinning it.

**EC-2.** The `rust_imports.rs` doctest carries a reference crate path today
and is the reason edit class 5 exists. If other doctests surface at move
time, they take the same rewrite, and the review notes count them.

**EC-3.** Shebang-based language detection (`language.rs`) treats unknown
interpreters as text tier. Moves as is.

## Tightening candidates, flagged not decided

1. The seven ArangoDB doc mentions (six in `lsp/edges.rs`, one in
   `symbols.rs`) and the small HADES set (`language.rs`, `symbols.rs`,
   `lsp/session.rs` docs). Spec-mandated correction, lands in tightening.
2. `"__hades_readiness_probe__"` in `lsp/session.rs`, a probe query string
   sent to the local language server. Behavior-neutral to rename, but it is a
   string a debugging session might grep server logs for, so the rename is a
   decision, not housekeeping.
3. Test fixture strings such as the `/tmp/hades source/` path with a Greek
   lambda filename in the URI test. Cosmetic, decide at
   review.

## Implementation Notes

### DO

- Verify the move mechanically: a scripted diff against the reference must
  show only the six allowed edit classes, and the review notes report the
  count of changed lines per class.
- Keep per-file review notes, as every lift has.
- Run the self-application e2e before the PR leaves draft: full analysis over
  this repository and the reference tree, syn path minimum, reporting file,
  symbol, edge, and tier counts. This is the corpus test the PRD names, and
  it needs no analyzer beyond what the build already has.
- Correct the PRD dependency table in passing, citing this spec.

### DON'T

- **Do not refactor.** 9,294 lines invite it everywhere. The move commit is
  the only diff a reviewer can trust mechanically, and every in-transit edit
  taxes it.
- **Do not extract vocabulary.** G4, standing.
- **Do not touch emitted field names or serde renames.** Sink wire shape.
- **Do not make the probe tests required.** Skip-not-fail is the contract.
- **Do not fix defects in transit.** Record them, fix above the move with
  review visibility, the workflow every lift has used.

## Development Environment

```bash
cd /home/todd/git/Yeomna
cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check
```

Nothing requires Postgres, a service, or a GPU. rust-analyzer is present via
the pinned toolchain component, so `ra_span_agreement` exercises rather than
skips on this box. libclang and gopls may or may not be present, and both
outcomes are valid test runs.

## Success Criteria

1. Build, test, clippy, fmt clean from the repository root at merge.
2. The scripted diff shows only the six allowed edit classes in the move
   commit.
3. All 124 moved unit tests pass, and the three probes pass or skip per
   their analyzers' presence.
4. The self-application e2e reports clean analysis of both corpora, numbers
   in the PR.
5. Store-reference grep (`arango`, `sql`, `postgres`) clean at merge, and
   `hades` grep clean except provenance citations, both after tightening.
6. Review notes account for every file and every edit-class count.

## QA Acceptance Criteria

1. `containers.rs` names match the reference's `CODEBASE` static verbatim,
   and its docs say the sink strips the prefix.
2. `ra_span_agreement` runs green on this box (rust-analyzer is pinned into
   the toolchain), guarding the keys line-identity contract.
3. The Issue and PR follow the established pattern, with the e2e numbers
   posted before undraft.
