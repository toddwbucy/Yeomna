# Review Notes: yeomna-embed lift

Reviewer: Claude, with Todd. Date: 2026-08-11. Both source files were read in
full at spec time.

## Accounting

| Source (`crates/hades-core/src/persephone/`) | Destination | Lines | Diff class |
|---|---|---|---|
| `mod.rs` | `src/lib.rs` | 17 | Provenance note, Persephone branding dropped per the naming ruling, PE-API doc pointer generalized |
| `embedding.rs` | `src/embedding.rs` | 713 | Byte-identical at the move commit |
| `extraction.rs` | `src/extraction.rs` | 337 | Five `hades_proto` to `yeomna_proto` import rewrites, then a rustfmt re-sort of the use block as a separate commit |

1,067 lines, five endpoint-parsing tests moved and passing, no test touches a
live service.

## What the review found

**The embedding client never touches the proto.** It is pure HTTP through the
hyper stack, confirming at compile time what spec 003's trimming section
measured: the gRPC embedding protocol is dead and only the extraction client
consumes `yeomna-proto`.

**Preserved working behavior worth knowing:** client-side request batching
with GPU-OOM halving retry, a 600 second default timeout sized for large
PDFs, and endpoint parsing that accepts `http://`, `https://`, `unix://`,
and absolute socket paths. All moved unchanged per FR-E3.

**`extraction.rs` arrives with zero tests**, recorded per FR-E4. Candidates
for tightening are construction and endpoint tests only, never integration
tests against a running extractor.

**The tonic UDS idiom** (dummy URI plus a Unix connector through
`service_fn`) is correct and looks wrong, per EC-2. Do not fix it.

## Tightening candidates flagged by the spec, standing for the PR

1. `/run/hades/extractor.sock` as the extraction default path.
2. `DEFAULT_ENDPOINT_URL = "http://localhost:8087/v1"`, the TCP embedder
   default. The `unix://` capability already exists, so this is a default,
   not a limitation.
3. HADES and Persephone branding in `embedding.rs` and `extraction.rs` doc
   comments, corrected as documentation.

## Verification

- Workspace gate: 92 unit tests plus 8 doctests across six crates, clippy
  clean, fmt clean.
- `diff` against the reference: `embedding.rs` byte-identical at the move
  commit, `extraction.rs` differs by the five import rewrites (plus the
  follow-up rustfmt re-sort), `lib.rs` rewritten as documented.
- `grep -ri arango crates/yeomna-embed`: zero matches.
