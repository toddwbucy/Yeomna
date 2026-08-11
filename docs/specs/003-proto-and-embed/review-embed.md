# Review Notes: yeomna-embed lift

Reviewer: Claude, with Todd. Date: 2026-08-11. Both source files were read in
full at spec time.

## Accounting

| Source (`crates/hades-core/src/persephone/`) | Destination | Lines | Diff class |
|---|---|---|---|
| `mod.rs` (17 lines) | `src/lib.rs` (19 lines) | 19 | Rewritten as documented: provenance note, Persephone branding dropped per the naming ruling, PE-API doc pointer generalized |
| `embedding.rs` | `src/embedding.rs` | 713 | Byte-identical at the move commit |
| `extraction.rs` | `src/extraction.rs` | 337 | Five `hades_proto` to `yeomna_proto` import rewrites, then a rustfmt re-sort of the use block as a separate commit |

1,069 lines at the destination (1,067 at the source, the rewritten lib.rs
being two lines longer), five endpoint-parsing tests moved and passing, no
test touches a live service.

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

## Tightening candidates, resolved in the CodeRabbit round

1. `/run/hades/extractor.sock` renamed to `/run/yeomna/extractor.sock`,
   review and the naming ruling converging on the FR-B4 precedent.
2. `DEFAULT_ENDPOINT_URL = "http://localhost:8087/v1"`, the TCP embedder
   default, **stands open**. The `unix://` capability exists, and the
   default moves when the config-file ruling or the SPU contract gives it
   somewhere real to point.
3. HADES and Persephone branding swept from both clients' docs, provenance
   citations in `lib.rs` retained.

## CodeRabbit round, beyond the candidates

Fixed: the request timeout now covers body collection, `https://` endpoints
are rejected at parse time with a clear message (no TLS connector exists,
and inside the sealed appliance TLS absence is design), the `file_name`
span field is recorded rather than declared and empty, and empty extraction
gained its own error variant distinct from a malformed response.

Declined: deduplicating the model-lookup predicate in `info()`. The spec's
DON'T list forbids polishing the hyper plumbing, since the SPU path
replaces it wholesale and polish spent here is discarded later.

## Verification

- Workspace gate: 92 unit tests plus 8 doctests across six crates, clippy
  clean, fmt clean.
- `diff` against the reference: `embedding.rs` byte-identical at the move
  commit, `extraction.rs` differs by the five import rewrites (plus the
  follow-up rustfmt re-sort), `lib.rs` rewritten as documented.
- `grep -riE 'arango|postgres|[^a-z_]sql' crates/yeomna-embed/src/` returns
  zero matches, which is the full store-and-vendor sweep behind the zero
  store coupling claim, not an ArangoDB-only check.
