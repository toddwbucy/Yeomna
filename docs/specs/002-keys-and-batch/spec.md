# Specification: 002 Keys and Batch

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 2.
Status: draft, 2026-08-10.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

Lift the two remaining leaf modules into their own crates: `yeomna-keys`
(deterministic key derivation, 493 lines) and `yeomna-batch` (resumable,
fault-isolated batch processing, 1,220 lines). Neither depends on the other,
neither depends on anything internal, and both were read in full for this spec.

Each crate follows the per-crate lift workflow: its own Issue, its own branch,
its own draft PR whose first commit is the verbatim move, CodeRabbit findings
as separate commits, e2e, then merge.

## Workflow Type

Two lifts. Each is a move plus the minimal mechanical edits named below, then
tightening commits on the PR.

## Sequencing Constraint

The workspace root (`Cargo.toml`, `rust-toolchain.toml`) lives on
`lift/001-chunking` until PR #2 merges. **Both Phase 2 branches cut from main
after that merge.** Branch names: `lift/002-keys` and `lift/002-batch`. Both
add themselves to `[workspace] members`, which will conflict trivially if both
are open at once. Whichever merges second rebases, and the conflict is one
line.

## Task Scope

### This Task Will

1. Create `crates/yeomna-keys` from `db/keys.rs`, a single-file crate.
2. Create `crates/yeomna-batch` from `batch/` (six files).
3. Move all tests with the code: 17 in keys plus doctests, 27 across batch.
4. Add both crates to the workspace and to `[workspace.dependencies]` as they
   land.
5. Leave build, test, clippy, and fmt clean at each merge.

### Out of Scope

- **`yeomna-code`, `yeomna-proto`, `yeomna-embed`, `yeomna-pipeline`.** Later
  phases.
- **Any store trait, table, or SQL.**
- **Changing any derived key value.** The determinism contract is absolute.
  See FR-K2, which freezes behavior that the tightening pass must not touch.
- **New chunk or edge semantics.** This is transport, not design.

## Files to Create

| Path | Source |
|---|---|
| `crates/yeomna-keys/Cargo.toml` | New |
| `crates/yeomna-keys/src/lib.rs` | `crates/hades-core/src/db/keys.rs` (493 lines) |
| `crates/yeomna-batch/Cargo.toml` | New |
| `crates/yeomna-batch/src/lib.rs` | `batch/mod.rs` (17 lines) |
| `crates/yeomna-batch/src/error.rs` | `batch/error.rs` (28) |
| `crates/yeomna-batch/src/processor.rs` | `batch/processor.rs` (583) |
| `crates/yeomna-batch/src/progress.rs` | `batch/progress.rs` (228) |
| `crates/yeomna-batch/src/rate_limit.rs` | `batch/rate_limit.rs` (143) |
| `crates/yeomna-batch/src/state.rs` | `batch/state.rs` (221) |

## Files to Reference

| What | Where |
|---|---|
| Key derivation source | `~/olympus/HADES-Burn/crates/hades-core/src/db/keys.rs` |
| Batch source | `~/olympus/HADES-Burn/crates/hades-core/src/batch/` |
| Workflow and commit discipline | CLAUDE.md, per-crate lift workflow |
| Prior art for the move shape | `docs/specs/001-workspace-and-chunking/` spec and review |

## Dependencies, measured by reading

The PRD's table understated these. Corrected here from the source, since `use`
line greps miss attribute macros and inline paths:

| Crate | Dependencies | Dev-dependencies |
|---|---|---|
| `yeomna-keys` | `regex`, `sha2` | none |
| `yeomna-batch` | `serde`, `serde_json`, `thiserror`, `chrono`, `tokio`, `tracing` | `tempfile` |

Versions go in `[workspace.dependencies]`, matching the reference's root
manifest where a pin exists there.

## Allowed edits in the move commit

The move commit is verbatim except for edits that are mechanically required to
compile in the new location. For these crates that means:

1. `mod.rs` to `lib.rs` renames and module wiring.
2. **Doctest paths in `keys.rs`.** The doc examples import
   `hades_core::db::keys::...` and doctests compile against the real crate, so
   they are rewritten to `yeomna_keys::...`. This is the same class of edit as
   an import rewrite and it is confined to `#` doctest lines and `use` lines
   inside examples.
3. The provenance note in crate docs.

Nothing else. In particular the string `ArangoDB` survives the move commit in
doc comments, and that is expected. It gets corrected in a tightening commit.

## Requirements: yeomna-keys

**FR-K1.** Public API unchanged: `normalize_document_key`, `strip_version`,
`chunk_key`, `embedding_key`, `file_key`, `symbol_key`, `edge_key`,
`compliance_edge_key`, `model_hash`. (The last is scheduled for removal in a
tightening commit, see FR-K4. It rides the move first.)

**FR-K2. Determinism is frozen, and the reasons have changed.** The 254-byte
cap, the character sanitization set, the `::` to `__` rewrite, the hash8
suffix, and the fixed-width u64 line encoding were all built to satisfy
ArangoDB's `_key` rules. ArangoDB is gone. The behaviors stay, because the
derived keys are Yeomna's idempotency contract: re-ingest produces the same
keys or `ON CONFLICT` upserts stop matching. The tightening pass may rewrite
every doc comment and may not change one byte of output. Any future proposal
to "simplify now that ArangoDB is gone" is answered by this paragraph.

**FR-K3. Golden-value tests are added in a tightening commit.** The moved
tests pin prefixes and properties. Add full-literal pins for at least:
`file_key("core/models.py")`, `chunk_key("2501_12345", 3)`,
`embedding_key("2501_12345_chunk_3")`, `normalize_document_key("2501.12345v2")`,
and the complete 8-hex-suffix outputs of `symbol_key("src_lib_rs",
"Config::new", 12)` and `edge_key` on a known triple, captured from the moved
code's own output at review time. These make silent drift loud.

**FR-K4. `compliance_edge_key` is removed in a tightening commit, not in the
move.** It builds keys linking documents to `smell_specs`, and smell is
methodology, dropped from the port by the closed ruling. Its only consumers
are smell commands that are not coming. The removal commit cites the ruling,
and the move commit keeps the file verbatim so the deletion is visible in
history rather than hidden inside a copy.

**FR-K5.** Every remaining public function keeps its doctest, rewritten to the
new crate path.

### Edge cases, keys

**EC-K1.** `symbol_key` with same qualified name and different lines yields
distinct keys (issue #148 in the reference). Test moves with the code.

**EC-K2.** Overlong file keys fold into the hash and cut at char boundaries.
Non-ASCII file keys must not panic. Tests move with the code.

**EC-K3.** `normalize_document_key` strips only a **trailing** `v\d+`.
`v2_doc.key` keeps its prefix. Test moves with the code.

## Requirements: yeomna-batch

**FR-B1.** Public API unchanged: `BatchProcessor`, `BatchProcessorConfig`,
`BatchSummary`, `ItemResult`, `BatchError`, `ItemError`, `ProgressEvent`,
`ProgressReporter`, `ProgressStatus`, `RateLimiter`, `BatchState`.

**FR-B2.** The state file format stays wire-identical: `completed` as a list,
`failed` as a map of id to message, optional RFC 3339 timestamps. The
`test_python_compat` test pins this and moves with the code. The doc comment
crediting the retired Python CLI gets updated in tightening, the format does
not.

**FR-B3.** Atomic save semantics (write tmp, rename) are preserved.

**FR-B4. The default state filename is a tightening decision, flagged here.**
`BatchProcessorConfig::default()` writes `.hades-batch-state.json`. No Yeomna
deployment exists, so renaming to `.yeomna-batch-state.json` costs nothing
today and stops costing nothing the day someone ships. Recommended: rename in
a tightening commit. The move commit keeps the old name.

**FR-B5.** Semantics worth stating because they surprise: `skip_set` includes
**failed** items, so a resumed batch does not retry failures, it skips them.
`reset` exists to clear that. This is behavior to preserve and document, not
a bug to fix.

### Edge cases, batch

**EC-B1.** Missing state file loads as `None`, corrupt state file errors.
Tests move with the code.

**EC-B2.** `RateLimiter::new(0.0, n)` means unlimited, and backoff then uses a
one-second floor. Tests move with the code.

**EC-B3.** Timing-based tests (`test_rate_limit_acquire` asserts at least
80ms elapsed) can flake on a loaded machine. If CI flakes, loosen in a
tightening commit with a comment, never in the move.

## Implementation Notes

### DO

- One Issue and one draft PR per crate, verbatim move as commit 1.
- Read every file again at move time and keep per-file review notes beside
  this spec, as `review-keys.md` and `review-batch.md`.
- Capture the golden values for FR-K3 from the moved code before any
  tightening lands, so the pins predate the first edit.
- Correct the PRD's dependency table for batch in passing, citing this spec.

### DON'T

- **Do not change any derived key, ever.** FR-K2.
- **Do not remove `compliance_edge_key` in the move commit.** FR-K4.
- **Do not port `db/collections.rs` or anything else from `db/`.** Only
  `keys.rs` leaves that directory, and it leaves it as its own crate rather
  than recreating a `db` module.
- **Do not add a store dependency to batch.** `ItemError.stage` mentions
  "storage" as an example string, and that is as close to a store as this
  crate gets.
- **Do not fix the ASCII-only or timing quirks silently.** Tightening commits
  with review visibility, or not at all.

## Development Environment

```bash
cd /home/todd/git/Yeomna
cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check
```

Nothing here needs Postgres, the embedder, or any service. `yeomna-batch`
tests touch the filesystem through `tempfile` only.

## Success Criteria

Per crate, at merge time:

1. Build, test, clippy, fmt all clean from the repository root.
2. The move commit diffs against the reference as: byte-identical files, or
   module wiring plus doctest-path rewrites plus the provenance note, nothing
   else.
3. Golden-value tests exist for keys (FR-K3) and pass.
4. `compliance_edge_key` is gone from the merged `yeomna-keys`, removed by a
   commit citing the methodology ruling (FR-K4).
5. Store-reference grep (`arango`, `sql`, `postgres`, case-insensitive) is
   clean **at merge**, after the doc-comment tightening. It is expected dirty
   at the move commit, and that is not a failure.
6. All tests pass with the Yeomna Postgres cluster stopped, by construction
   or by demonstration.
7. Review notes exist per crate and account for every file.

## QA Acceptance Criteria

1. Keys: all 17 moved tests plus doctests pass, plus the new golden pins.
2. Keys: a grep for `compliance` in the merged crate returns nothing.
3. Batch: all 27 moved tests pass, `test_python_compat` unmodified.
4. Batch: `skip_set` semantics documented in the crate docs by the tightening
   pass (FR-B5).
5. Both: the Issue documents the move and the PR body links spec and review
   notes, matching the 001 pattern.
