# Specification: 003 Proto and Embed

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 3.
Status: draft, 2026-08-11.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

Lift the embedder path: `yeomna-proto` (generated protobuf for the extraction
service protocol) and `yeomna-embed` (the HTTP embedding client and the gRPC
extraction client, from `persephone/`, 1,067 lines). Every file was read for
this spec, and the reading changed the scope in one important way, recorded
under Trimming.

These are clients for services that are currently down. Nothing in this spec
requires a running embedder or extractor, and no test may either.

## Workflow Type

Two lifts, sequenced. `yeomna-proto` is small and merges first. `yeomna-embed`
depends on it and cuts from main after that merge, so no stacked branches and
no workspace conflict.

## Trimming: one proto package comes over, three stay behind

The reference's `hades-proto` compiles four packages. Measured consumers, from
reading rather than assuming:

| Package | Consumers outside the proto crate's own test |
|---|---|
| `persephone.extraction` | `persephone/extraction.rs`, in lift scope |
| `persephone.embedding` | **None.** Dead wire protocol from the gRPC embedder that PR #70 deleted. The live embedding client speaks HTTP (PE-API shape) |
| `persephone.common` | **None.** No proto file imports it either |
| `hades.training` | `training.rs`, `graph/export.rs`, `hades-prefetch`, all outside lift scope |

**`yeomna-proto` compiles `extraction.proto` only.** This is the no-vocabulary
ruling applied to wire schemas: a protocol with no consumer is not lifted, and
each of the three left behind can be added the day a consumer arrives, since
the source protos stay readable in the reference. The dead embedding protocol
in particular must not come over, for the same reason the dead ANN module did
not.

**Proto package and service names are wire contract and stay verbatim.** The
deployed Python extractor implements `persephone.extraction.ExtractionService`.
Renaming the package in the `.proto` would change the gRPC method paths and
silently break the only working service this client can talk to. The crate is
named `yeomna-proto`, the package inside it stays `persephone.extraction`, and
the doc comments say why.

## Task Scope

### This Task Will

1. Create `crates/yeomna-proto`: `build.rs`, a trimmed `lib.rs`, the
   `proto/persephone/extraction/extraction.proto` file copied verbatim into a
   workspace-root `proto/` directory, and the extraction-relevant parts of the
   reference's `proto_types.rs` integration test.
2. Create `crates/yeomna-embed` from `crates/hades-core/src/persephone/`:
   `mod.rs` to `lib.rs`, `embedding.rs`, `extraction.rs`.
3. Rewrite `hades_proto::` imports to `yeomna_proto::`.
4. Leave build, test, clippy, fmt clean at each merge.

### Out of Scope

- **The PE-API fork ruling and the SPU embedder contract.** The spike
  (`spikes/jina-late-loop/`) produced the evidence, and the ruling belongs to
  the joint contract document with WeaverTools, not to a client lift. This
  lift moves the client that speaks the OpenAI single-vector shape to the
  Python service, because that is what exists.
- **Late-chunking wiring.** New construction, downstream of the contract.
- **The Python services themselves.** Phase 6.
- **Config-file migration for endpoints.** Raised 2026-08-10 and not ruled.
  The lifted defaults are flagged below as tightening candidates instead.
- **`persephone.embedding`, `persephone.common`, `hades.training` protos.**
  Per Trimming.
- **`yeomna-code`, `yeomna-pipeline`.** Phases 4 and 5.

## Files to Create

| Path | Source | Lines |
|---|---|---|
| `proto/persephone/extraction/extraction.proto` | `proto/persephone/extraction/extraction.proto`, verbatim | 143 |
| `crates/yeomna-proto/build.rs` | `crates/hades-proto/build.rs`, trimmed to one proto | ~30 |
| `crates/yeomna-proto/src/lib.rs` | `crates/hades-proto/src/lib.rs`, trimmed | ~12 |
| `crates/yeomna-proto/tests/proto_types.rs` | extraction-relevant parts of the reference's | part of 170 |
| `crates/yeomna-embed/src/lib.rs` | `persephone/mod.rs` (17) | 17 |
| `crates/yeomna-embed/src/embedding.rs` | `persephone/embedding.rs` | 713 |
| `crates/yeomna-embed/src/extraction.rs` | `persephone/extraction.rs` | 337 |

## Dependencies, measured by reading

The PRD table for `persephone/` was incomplete, the fourth correction in this
pattern. Full set from the source:

| Crate | Dependencies |
|---|---|
| `yeomna-proto` | tonic, prost, prost-types (build: tonic-build) |
| `yeomna-embed` | yeomna-proto, tonic, hyper, hyperlocal, hyper-util, http, http-body-util, tower, serde, serde_json, thiserror, tokio, tracing |

Versions match the reference's root manifest: tonic 0.13, prost 0.13,
hyper 1, hyperlocal 0.9.

**Build environment:** `tonic-build` invokes `protoc` at build time. The box
has libprotoc 35.1 at `/usr/bin/protoc`. Generated code is not committed, so
a protoc major change could alter it silently. Noted as an environment fact,
and worth a line in the eventual pinning conversation, not action now.

## Requirements

**FR-P1.** `yeomna-proto` exposes `pub mod extraction` with client stubs and
server traits generated from the verbatim proto file, nothing else.

**FR-P2.** The moved integration test covers the extraction types it covered
in the reference, and the parts covering trimmed packages are dropped with a
note in the review, not silently.

**FR-E1.** `yeomna-embed` public API is unchanged from the reference's
`persephone` module: `EmbeddingClient`, `EmbeddingClientConfig`, `EmbedResult`,
`ProviderInfo`, the extraction client types, under the same names.

**FR-E2.** Endpoint parsing behavior is preserved exactly: `http://` and
`https://` as TCP base URLs, `unix://` and absolute paths as Unix sockets,
anything else rejected. The five moved tests pin it.

**FR-E3.** The embedding client's OOM-halving retry, client-side batching, and
error taxonomy move unchanged. They are working behavior against the real
service, and the contract work that replaces this path will want them as
reference.

**FR-E4.** No test requires a live service. `extraction.rs` arrives with zero
tests, which the review records, and any tests added in tightening are
construction and endpoint-parsing tests, not integration tests against a
running extractor.

### Edge cases

**EC-1.** `parse_endpoint` on garbage input returns the error message the
reference produced, pinned by the moved tests.

**EC-2.** The extraction client's tonic `Endpoint` for a Unix socket uses a
dummy URI with a UDS connector. This is tonic idiom, looks wrong, and is
right. The review notes say so, so nobody fixes it.

## Tightening candidates, flagged not decided

Decisions for the PR conversation, following the FR-B4 precedent that renames
cost nothing while no deployment exists:

1. **`/run/hades/extractor.sock`** as the extraction default. The Phase 6
   service will bind somewhere Yeomna-owned. Renaming the client default now
   versus coordinating with Phase 6 is the decision.
2. **`DEFAULT_ENDPOINT_URL = "http://localhost:8087/v1"`**, a TCP default for
   the embedder client. The known compromise, on the client side. The unix://
   path support already exists, so this is a default, not a capability.
3. **HADES branding in doc comments**, corrected as documentation in
   tightening, store-grep and hades-grep clean at merge except provenance
   citations.

## Implementation Notes

### DO

- One Issue and one draft PR per crate, verbatim move as the first commit,
  same as lifts 001 and 002.
- Copy the proto file byte-identically and diff it in the review.
- Correct the PRD dependency table for `persephone/` in passing, citing this
  spec.
- Keep per-crate review notes beside this spec.

### DON'T

- **Do not rename anything inside the `.proto`.** Wire contract.
- **Do not lift the three unconsumed proto packages.**
- **Do not add integration tests that need a running service.**
- **Do not resolve the PE-API fork here**, in code, in defaults, or in doc
  comments. The client speaks what the Python service speaks until the
  contract document rules.
- **Do not refactor the hyper plumbing.** It is verbose and it works, and the
  SPU path will replace it wholesale, so polish spent here is discarded
  later.

## Development Environment

```bash
cd /home/todd/git/Yeomna
cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check
```

Requires `protoc` on PATH for `yeomna-proto`'s build script. Nothing requires
Postgres, the embedder, the extractor, or the GPU.

## Success Criteria

Per crate, at merge:

1. Build, test, clippy, fmt clean from the repository root.
2. `extraction.proto` byte-identical to the reference's copy.
3. `yeomna-proto` generates exactly one package.
4. `yeomna-embed`'s move commit diffs as module wiring, `hades_proto` to
   `yeomna_proto` import rewrites, and the provenance note, nothing else.
5. The five endpoint-parsing tests pass unchanged.
6. Store-reference grep clean at merge. `hades` grep clean at merge except
   provenance citations and, pending the tightening decision, the default
   socket path.
7. Review notes exist per crate.

## QA Acceptance Criteria

1. Proto: the trimmed `proto_types.rs` passes, and the review names which
   test sections were dropped with the trimmed packages.
2. Embed: all moved tests pass, no new dependency on a live service.
3. Both: Issue and PR follow the 001/002 pattern.
