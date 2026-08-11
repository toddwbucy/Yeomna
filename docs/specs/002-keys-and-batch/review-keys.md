# Review Notes: yeomna-keys lift

Reviewer: Claude, with Todd. Date: 2026-08-10. The file was read in full both
at spec time and again at move time.

## Accounting

| Source | Destination | Lines | Diff class |
|---|---|---|---|
| `crates/hades-core/src/db/keys.rs` | `crates/yeomna-keys/src/lib.rs` | 493 | Move plus doctest paths plus provenance note |

Test counts corrected against the spec: the spec estimated 17 moved tests.
The true count at move time was 15 unit tests plus 8 doctests. After
tightening: 21 unit tests (six golden pins added) plus 8 doctests. The
`compliance_edge_key` doctest was removed with its function in commit 2,
dropping the count to 7, and the CodeRabbit round restored it to 8 by adding
the `model_hash` doctest FR-K5 wanted, pinning the same full SHA-256 literal
as the golden test.

## Commit structure

1. **Move** (verbatim plus allowed edits): doctest paths
   `hades_core::db::keys` to `yeomna_keys`, provenance note. `ArangoDB`
   survives this commit in prose by design.
2. **Remove `compliance_edge_key`** (FR-K4): smell methodology, dropped by
   the closed ruling. Deleted above the move so history shows the deletion.
3. **Golden pins** (FR-K3): six tests with full-literal outputs including
   complete hash suffixes, captured from the moved code before any edit:
   `symbol_key("src_lib_rs", "Config::new", 12)` is
   `src_lib_rs__Config__new__086f8847`, and the `edge_key` and `model_hash`
   literals are pinned alongside.
4. **Doc rewrite** (FR-K2): behavior explained by the frozen contract rather
   than the dead store. No code byte changed. Goldens passing across this
   commit is the proof.

## What the review found

**The dangerous-looking code is the contract.** The sanitization keep-set,
the 254-byte cap, the newline-separated hash inputs, and the fixed-width u64
line encoding all look like candidates for cleanup now that no store enforces
them. Every one of them is load-bearing for idempotent re-ingest, and FR-K2
plus the golden pins exist to stop that cleanup.

**Architecture note preserved from the reference:** the u64 line encoding
exists so keys are identical across 32-bit and 64-bit hosts. Kept, and worth
keeping, since an appliance's architecture is not promised.

**One reference-only artifact:** `symbol_key` docs cite the reference's
`tests/ra_span_agreement.rs`, which does not come over until `yeomna-code`
lands in Phase 4. The citation stays as history until then.

## Verification

- 21 unit tests plus 7 doctests passing, clippy clean, fmt clean.
- Store-reference grep (`arango`, `sql`, `postgres`): clean after commit 4,
  dirty between commits 1 and 3 as the spec says to expect.
- `grep -c compliance`: zero after commit 2.
- Golden values captured pre-tightening and passing post-tightening.
