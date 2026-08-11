# Review Notes: yeomna-code lift

Reviewer: Claude, with Todd. Date: 2026-08-11.

## Accounting

21 source files moved 1:1 (`mod.rs` to `lib.rs`), plus three probe tests from
the reference's integration directory, plus two files the move created:
`containers.rs` (transitional names, values verbatim from the reference's
`CODEBASE` static) and `canonical_json.rs` (63 lines, carried byte-identical,
see below). 9,294 source lines at the origin.

Scripted diff against the reference, counted by edit class:

| Class | Lines |
|---|---|
| Combined `db::{collections::CODEBASE, keys}` rewrites | 3 sites, 6 lines |
| `db::collections::CODEBASE` alone | 2 |
| `db::keys` alone (aliased so call sites stay identical) | 2 |
| `db::keys::{file_key, symbol_key}` function import | 1 |
| `crate::chunking` to `yeomna_chunking` | 1 |
| Intra-crate paths (`crate::code::` to `crate::`) | 4 |
| Doctest crate paths | 1 in src, 4 in tests |
| **Total changed lines across moved files** | **31** |

The test-fixture strings in `rust_imports.rs` tests (lines carrying
`crate::db::keys` and `crate::db::Pool` as string literals) are resolver test
data and were verified untouched, the exact trap the spec named.

## The compiler-forced class, sixth dependency correction

`chrono` (inline paths in lsp symbol timestamps) and `crate::canonical_json`,
a hidden internal module of `hades-core` that `cpp.rs` and `tree_sitter.rs`
hash symbol metadata through. Both were invisible to the `use`-line survey.
`canonical_json.rs` is order-independent JSON encoding feeding symbol content
hashes, so it is identity-relevant and was carried byte-identical rather than
rewritten. `tempfile` joined as a probe dev-dependency.

## Test results

- 125 unit tests moved and passing (the spec estimated 124).
- `ra_span_agreement`: **exercised, green** on this box, since rust-analyzer
  is pinned into the toolchain. This is the cross-crate line-identity
  contract between `yeomna-code` and `yeomna-keys`, tested for real.
- `gopls_semantic`: passing per its own skip-or-run contract.
- `clang_cuda_probe`: **capability gap found and recorded.** libclang 22.1.8
  analyzes the CUDA fixture at semantic tier (33 symbols) but attaches no
  calls metadata, where the clang the probe was written against resolved the
  kernel launch. The probe now skips on absent calls with the gap stated,
  and still fails hard on present-but-wrong calls. Standing item: .cu
  ingests on this box will lack kernel-launch edges until a compilation
  database is provided or the clang crate's version features are revisited.

## Tightening applied

- Seven ArangoDB doc mentions corrected to sink and transitional-container
  framing.
- HADES branding in docs rewritten to Yeomna.
- One behavior rename under the naming ruling and FR-B4 precedent:
  `YEOMNA_TOOLS_DIR` falling back to `~/.local/share/yeomna/tools` for the
  managed-analyzer directory. Nothing deployed sets the old env var or fills
  the old path.
- Flagged candidates left for the review conversation, per spec:
  `"__hades_readiness_probe__"` (a probe string a debugging session might
  grep server logs for) and the `/tmp/hades source/` URI test fixture.

## Self-application e2e

The corpus test the PRD names, run before undraft:

| Corpus | Files | Analyzed | Symbols | Top-level defs | Tier |
|---|---|---|---|---|---|
| Yeomna (this repo) | 45 | 45 | 1,043 | 276 | all semantic |
| HADES-Burn (reference tree) | 149 | 149 | 3,359 | 1,011 | all semantic |

Zero failures, zero unsupported files, both corpora fully at semantic tier.
The extractor analyzed its own source and the codebase it was lifted from.

## Verification

- Workspace gate: all crates green, clippy clean, fmt clean.
- `grep -rn 'ArangoDB|HADES|hades' src tests` clean except the provenance
  citation in `lib.rs` and the two flagged candidates above.
- `containers.rs` values match the reference's `CODEBASE` static verbatim.
