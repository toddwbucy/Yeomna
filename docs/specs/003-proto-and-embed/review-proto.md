# Review Notes: yeomna-proto lift

Reviewer: Claude, with Todd. Date: 2026-08-11.

## Accounting

| Path | Source | Diff class |
|---|---|---|
| `proto/yeomna/extraction/extraction.proto` | `proto/persephone/extraction/extraction.proto` | Two lines: the `package` declaration and its header comment, per the naming ruling. Everything else byte-identical, shown by diff |
| `crates/yeomna-proto/build.rs` | `crates/hades-proto/build.rs` | Trimmed from four protos to one, path updated |
| `crates/yeomna-proto/src/lib.rs` | `crates/hades-proto/src/lib.rs` | Trimmed from four modules to one, docs record the trim and the rename |
| `crates/yeomna-proto/tests/proto_types.rs` | reference's `proto_types.rs` | `test_extraction_types` moved verbatim (imports rewritten). `test_common_types`, `test_embedding_types`, `test_training_types` dropped with their trimmed packages, per FR-P2 |

## What the review found

**The trim held up at compile time.** One package generates, the extraction
test exercises requests, responses, enums, and maps against the generated
types, and nothing referenced the three packages left behind.

**The rename is two lines.** The `package yeomna.extraction` declaration and
its header comment. Service, rpc, message, and field names untouched, per the
ruling.

**protoc dependency confirmed live.** The build script ran against libprotoc
35.1. Generated code is not committed, so the protoc version is part of the
build environment. Recorded for the pinning conversation.

## CodeRabbit round

**`buf.yaml` restored**, byte-identical from the reference, after review
caught that the lift dropped it. Inert to the cargo build.

**`file_path` removal declined, recorded.** Review proposed removing the
caller-controlled `file_path` field from `ExtractRequest` as a path-traversal
surface. Declined on threat model: the extractor binds a Unix socket inside
the sealed appliance, callers are admitted by filesystem permission, and
charter section 9 rules out a network listener permanently, so any caller is
already inside the trust boundary and still bounded by the service's own OS
permissions. The dual path-or-content mode is deliberate co-resident design,
and removing the field would rewrite wire semantics the Phase 6 server must
speak. **Standing item:** if any front-end transport or third-party caller
ever fronts this service, `file_path` becomes a named security review item
before that opening ships.

## Verification

- Build, 1 integration test passing, clippy clean, fmt clean.
- `diff` against the reference proto shows exactly the two ruled lines.
- `grep -ri persephone` on the crate and proto: nothing except this note and
  the provenance citations in `lib.rs`.
