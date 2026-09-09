# Specification: 015 Native Extraction and the Document Graph

Owner: H5 (the holes ledger), under the pipeline-libraries PRD's
document flow. Ruled by R19 (2026-09-09, revising R5), with R19a
proposed below.
Status: draft, 2026-09-09. R19 agreed by Todd. R19a ruled the same
day in its v2 form after the dig into the corpus's own graph notation.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

Extraction goes native for declarative formats, and documents join the
graph. Three pieces:

1. An extraction trait at the orchestrator seam where the Python
   socket client sits today, with two implementors: the existing
   client, unchanged, and a new native backend built on the `docling`
   converter crate (from `docling-project/docling.rs`, version-pinned,
   PDF feature off).
2. The Markdown path end to end: `.md` files convert, chunk through
   the existing chunker, and land as document nodes with FTS-ready
   chunks, proven on the WeaverTools corpus (51k lines of load-bearing
   Markdown).
3. The `conforms:` resolver: the headers WeaverTools carries in source
   (492 measured 2026-09-09) become declared doc-to-code edges, which
   is the linkage that makes the knowledge graph semantic rather than
   two disconnected graphs.

## The rulings

**R19, agreed 2026-09-09 (revises R5).** Adopt the `docling` converter
crate only, pinned, as a native extraction backend for declarative
formats. Markdown proves it now, the office formats (DOCX, XLSX, PPTX,
RTF, EML, legacy Office) ride along untested until a corpus needs one,
each an afternoon's test per the H5 notes. The PDF pipeline is not
flipped: PR #17 stays in draft as the PDF behavioral reference until
the R5 spike (captions, formulas, parity on our documents) runs
against the Rust engine's PDF path. `docling-rag` is declined with
reasons on the record: its defaults are a second store (sqlite-vec), a
remote LLM (OpenRouter), a REST surface, and a foreign chunker that
would break the golden-key chunk contract. Three harvest pointers are
recorded in the ledger where their consumers will look: RRF hybrid
fusion (the `hybrid` flag, H4/H9), the deterministic hash embedder as
a test double (H4), and the converter-to-pipeline seam as reference
for this spec's trait.

What changed since R5: the format migration is complete and validated
byte-for-byte against Python docling for declarative formats, and
those formats need no ML assets, no GPU, and no model downloads. R5's
young-and-moving concern is answered by the pin, which is how this
house refuses cadence it does not want.

**R19a, ruled 2026-09-09: declared vocabulary is the source's own.**
The dig that settled this: the WeaverTools docs carry 387 fenced
`graph` blocks declaring 473 nodes and twelve edge relations
(`asserts`, `grounds`, `draws`, `defines`, `party`, `seam`, `parent`,
`holds`, and more) in their own notation, beside the 492 `conforms:`
headers in source. A semantic KG is installation-specific and grown
through use, so its vocabulary belongs to the operator and the corpus,
never to our DDL. Three clauses:

1. The `edges_declared` partition's CHECK opens to identifier shape,
   the same shape `edges_asserted` carries. The R18 principle
   generalizes: asserted relations are the caller's vocabulary,
   declared relations are the source's, and sources speak their own
   words. `edges_structural` keeps the closed five-relation list,
   because structural edges come from our analyzers and that
   vocabulary is ours to close. Analyzer-side hygiene for declared
   emissions moves to Rust, where the emitters already validate.
2. Corpus-declared graph nodes land as kind `document`, with the
   block's `kind:` and `tag:` preserved in payload. No change to the
   node kinds CHECK. This is the methodology disposition landing as
   ruled: smells and claims become documents and edges in the graph,
   reached through the verb layer.
3. `SCHEMA_VERSION` bumps to 1.2.0 and the cluster takes the
   documented no-migration path.

Considered and declined: adding `conforms` alone (strands the other
seven hundred declared edges in the same corpus), enumerating the
corpus's twelve relations into the CHECK (hard-codes one customer's
ontology and makes every future corpus a schema migration), reusing
`implements` (ambiguous with trait-impl edges), and new node kinds
per corpus (the same scaling flaw, and the ruled sentence already
assigns claims to `document`). What the substrate keeps: basis as the
trust boundary, `analyzer` and `basis` NOT NULL, unattributed edges
unrepresentable. The grammar is fixed and the language is the
operator's.

## Task Scope

- The `Extractor` trait in `yeomna-pipeline`, async, one method shaped
  by what `process_inner` consumes today (the `ExtractResult` the
  socket client returns). The socket client implements it by
  delegation. Behavior of the existing path does not change.
- `NativeExtractor` implementing the trait via the `docling` crate for
  declarative formats, refusing (typed, per-file) formats it does not
  handle rather than failing a batch, the H3 degradation discipline.
- A document walk for the ingest operation: `.md` files under a root,
  respecting ignore rules, hash-skipped on re-ingest so R8 and R9 hold
  for documents exactly as they do for code.
- The `conforms:` resolver: scan source files for the header
  convention (`//! conforms: <slug>` and `/// conforms: <slug>`,
  verified against the WeaverTools sources 2026-09-09, 492 sites, 358
  distinct slugs), resolve slugs to the claim nodes the graph-block
  resolver created, emit edges with relation `conforms`, basis
  `declared`, analyzer `conforms-header`. Unresolved targets are
  counted and reported, never fatal, and the summary names them, since
  a header pointing at a retired claim is a finding the graph owes its
  operator.
- The graph-block resolver: parse fenced `graph` blocks in Markdown
  (line-oriented, `node:`/`kind:`/`tag:`/`edge:`/`from:`/`to:`), emit
  one `document`-kind node per declaration (block kind and tag in
  payload) and one declared edge per edge stanza, analyzer
  `graph-block`. Endpoints resolve to existing nodes first, so a
  `from: weaver-types` lands on the code node the codebase ingest
  created, which is where the doc graph and the code graph fuse.
  An endpoint that resolves nowhere becomes a `document`-kind
  placeholder node, counted in the summary, because a declared edge
  to a thing not yet ingested is still a declaration.
- An `#[ignore]` operation test in the dogfood style, pointed
  read-only at `/opt/weavertools/WeaverTools`, ingesting docs plus
  code plus links into a scratch graph and printing the census.

## Out of Scope

- The PDF and image pipeline, its models, and the R5 spike. PR #17
  stays in draft.
- Office-format testing (no corpus yet), OCR, LaTeX source packages.
- Embeddings (H4), hybrid retrieval, anything from `docling-rag`.
- Repository connectors (charter 5.2, separate app).
- Ingestion verbs (H2 Phase 6 wraps this flow later).
- Edge history, new verbs, contract changes.

## Files to Modify

- `crates/yeomna-pipeline/src/orchestrator.rs`: the trait seam.
- `crates/yeomna-pipeline/src/extract.rs` (new): the trait and the
  native backend.
- `crates/yeomna-pipeline/src/documents.rs` (new, name at builder's
  discretion): the walk and the document ingest operation.
- `crates/yeomna-code` or `crates/yeomna-pipeline`: the `conforms`
  resolver, placed where the source-scanning tooling already lives.
- `crates/yeomna-store/schema.sql` and `SCHEMA_VERSION`: R19a.
- `crates/yeomna-pipeline/Cargo.toml`: the pinned `docling` dependency,
  default features audited, PDF off.
- Tests: a native-extraction suite, a conforms-resolver suite, the
  operation test.

## Files to Reference

- `crates/yeomna-pipeline/src/codebase.rs`: walk, hash-skip,
  degradation, summary shapes, the operation-test pattern.
- `crates/yeomna-embed` extraction client and `ExtractResult`: the
  shape the trait pins.
- `crates/yeomna-chunking`, `crates/yeomna-keys`: consumed unchanged,
  golden keys stay green.
- `docs/specs/011-codebase-orchestrator/` notes: the resolver-per-
  analyzer pattern the conforms resolver mirrors.

## Functional Requirements

- **FR1** The trait lands and the socket path is behaviorally
  unchanged: every existing pipeline test passes untouched.
- **FR2** A Markdown file converts natively: title and heading
  structure into the document payload, text into chunks through the
  existing chunker, `chunk_doc` bytes identical to what the same text
  through the old path would produce (the golden contract is the
  proof, not a parity harness).
- **FR3** Re-ingest is idempotent: unchanged files hash-skip, changed
  files log-on-change, `ingested_at` advances only on distinct payload.
- **FR4** The conforms resolver emits `conforms` edges, declared
  basis, from the code node carrying the header to the document node
  it names, with resolved and unresolved counts in the summary.
- **FR5** A format the native backend does not handle is a per-file
  typed refusal recorded in the summary, and the batch continues.
- **FR6** The WeaverTools operation test ingests the corpus read-only
  and reports nodes, chunks, edges, and conforms coverage.
- **FR7** The graph-block resolver emits the corpus's declared nodes
  and edges verbatim: relation names carried as written (identifier
  shape enforced), block kind and tag in payload, endpoints fused onto
  existing code nodes where keys resolve.

## Edge Cases

- **EC-1** A Markdown file with no headings: one document node, chunks
  from the body, title from the filename.
- **EC-2** A `conforms:` header naming a document that does not exist:
  counted unresolved, named in the summary, no edge, not fatal.
- **EC-3** An oversized file: the codebase pipeline's oversize
  discipline applies, counted and skipped.
- **EC-4** Two files normalizing to one document key: the second is a
  collision finding, reported, first wins, not fatal.
- **EC-5** Non-UTF8 bytes in a claimed-Markdown file: per-file typed
  refusal (FR5 shape).
- **EC-6** A document deleted from the corpus between ingests: this
  spec does not sweep it (retire is Phase 6), and the summary counts
  known-but-unseen documents so the operator can see the drift.
- **EC-7** A malformed graph-block line (one exists in the corpus
  today: a `kind:` whose value is a prose sentence): the block is a
  per-block typed refusal counted in the summary, the batch continues,
  and a relation or slug failing identifier shape is refused the same
  way rather than half-written.

## Implementation Notes

DO:

- DO pin the exact `docling` version and record the dependency-tree
  audit in the review notes.
- DO keep the chunk and key contracts byte-identical: the golden tests
  are the gate.
- DO verify the real `conforms:` header syntax against WeaverTools
  sources before writing the parser, on the grep-verify rule.
- DO run the gate three times before calling it done.

DON'T:

- DON'T enable the PDF feature, download models, or touch #17.
- DON'T import any docling-rag code or trait shapes.
- DON'T write a second chunker, embedder, or store path.
- DON'T let the resolver invent relations beyond `conforms` (the
  vocabulary grows by ruling, not by resolver).

## Success Criteria

1. The WeaverTools operation test ingests the Markdown corpus and the
   conforms linkage, with coverage reported against the 492 measured
   headers.
2. Golden keys and chunk contracts green, untouched.
3. Existing pipeline tests pass with the socket client behind the
   trait.
4. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- New suites cover FR1 through FR6 and EC-1 through EC-6, cluster-
  gated where they touch the store, per-cause skip messages.
- The no-SQL lint still passes: the new modules emit no SQL (the sink
  writes, as always).
