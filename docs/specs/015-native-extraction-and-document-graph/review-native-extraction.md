# Review notes: 015 native extraction and the document graph

Status: build complete 2026-09-09, awaiting CodeRabbit under the
three-exchange rule. R19, R19a, and R19b ruled the same day.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## The census (FR6), first run against the WeaverTools corpus

Code, documents, and links into one scratch graph, 18 seconds. This
table is the first run, at build time, with the corpus at WeaverTools
PR 528. The second run, after review round one and against the corpus
as it had grown, follows the table.

| | |
|---|---|
| Documents | 100 seen, 100 written, 0 failed, 0 oversized, 0 collisions |
| Chunks | 1518 |
| Graph blocks | 388 seen, 0 refused |
| Declared nodes | 474 written, 0 fused, 2 placeholders |
| Declared edges | 649 written, 0 rejected |
| Relations, as the corpus wrote them | asserts 396, grounds 81, draws 59, defines 44, party 23, seam 13, parent 12, holds 8, floor-link 8, writes 2, elects 2, reads 1 |
| Conforms | 175 source files scanned, 492 headers, 0 malformed, **492 edges written, 0 rejected, 0 unresolved** |
| The graph | 5147 edges |

Every header resolved to a claim the docs declared, and every file
carrying one existed as a code node, so the conforms pass is the
fusion: 492 declared edges from source files to the claims they answer
to. The two placeholders are endpoints the docs name and never declare,
reported by the operation as the spec requires.

The census found one more block and 13 `seam` edges where the 2026-09-09
survey's grep counted 387 and 12: the parser reads fences the grep's
line anchors missed. Re-run after CodeRabbit round one, against a
corpus that had grown in the meantime (WeaverTools merged its PR 529
at 13:57): 101 documents, 1546 chunks, 393 blocks with zero refusals, 480
declared nodes, 652 declared edges, and 498 headers deduplicated to
466 unique file-to-claim edges, the difference being item-level
restatements of a file-level header. 5238 edges. The corpus is a
moving target, which is the point of a graph that is grown rather than
dumped. The corpus line that reads `kind: the SPU still
decides nothing about what matters` sits outside any graph block, so
nothing was refused, and EC-7 is proven by the unit test that plants
the same line inside one.

## Build findings, and what they amended

**1. Kebab is admitted on the open partitions.** R19a ruled the declared
partition to identifier shape, and the corpus's relations include
`floor-link` while every claim slug is kebab. Sources speak their own
words, so the CHECK on `edges_declared` and `edges_asserted` admits the
hyphen (`^[a-z][a-z0-9_-]{0,62}$`), `check_relation` in the verb layer
mirrors it, and `document_graph::is_relation` mirrors both. Structural
stays closed. A refinement inside the ruling's own principle, not a
departure from it.

**2. The docling pin is load-bearing.** The crate published ten versions
in the two days before this build, five of them on the build day. R5's
young-and-moving concern is exactly right and the pin is exactly the
answer: `docling = "=1.41.0"`, default features off. Dependency audit:
the pipeline's normal tree grew from 229 to 326 unique crates (docling's
own subtree is 143, the rest shared). No ONNX, pdfium, or Whisper
anywhere in it. Build time for the crate with features off: nine
seconds wall clock.

**3. Placeholders never overwrite, and promotion is the sink's upsert.**
A placeholder is written with `overwrite: false`, so an endpoint that
exists in any form is left exactly as it is. Promotion needed nothing
new: the sink's node upsert already does `DO UPDATE SET kind =
EXCLUDED.kind, payload = EXCLUDED.payload`, so when the code ingest
arrives under the same key the id survives and every edge stays
attached. The integration test proves it in that order.

**4. Declarations fuse rather than clobber.** A graph block that names
an existing non-document node (the corpus describes its crates) leaves
that node alone and counts as fused. The probe grew `stored_kind` for
this and `stored_document_hash` for the hash-skip, two reads, the write
contract untouched.

**5. Conforms edges are file-granular.** The header sits on a file
(`//!`) or on an item (`///`). Both attach to the file node, with the
line recorded in the edge payload. Symbol-level attribution needs the
analysis's line ranges and is a refinement for when a query wants it.

**6. The document operation does not run through `Pipeline`.** The
orchestrator embeds unconditionally and no embedder ships (H4), so the
operation walks, converts, chunks, and writes on its own, reusing
`chunk_doc` so the chunk contract is byte-identical. `Pipeline` gained
the trait seam (generic over `Extractor`, default the socket client)
and nothing else, which is FR1.

**7. Skipped for writing, never for declaration (CodeRabbit round
one).** The first cut wrote the content hash with the document and only
then drew its declarations, so a run interrupted between the two, or
an edge the sink rejected, would have hidden behind the hash on the
next run. The fix mirrors the codebase orchestrator's rule that an
unchanged file still contributes its symbols: every file's graph blocks
are read before the hash-skip and re-declared on every run. Nodes
upsert with identical payloads and R8 keeps the history quiet, edges
upsert on their identity, and a missing declaration repairs itself on
the next run. The integration test deletes an edge between runs and
watches it return. Round one also refused reserved stanza keys as
extras (a missing blank line can no longer fold an edge into a node
silently), deduplicated restated nodes, edges, and headers with the
first in sorted order winning, gave the converter task its own
`Backend` error, and kept spec 005's `PipelineError::Extraction`
variant beside the seam's new one.

## What landed

- `extract.rs`: the `Extractor` trait, `ExtractError` (typed per file),
  the socket client behind it by delegation, `NativeExtractor` on
  docling with `MarkdownOutline` for title and headings.
- `document_graph.rs`: the fenced-block parser and the conforms scanner,
  pure, with shape checks mirroring the partition CHECKs.
- `documents.rs`: `ingest_documents` (walk, sort, hash-skip, convert,
  chunk, write, then nodes, placeholders, edges) and `link_conforms`.
- Schema 1.2.0 with the documented re-stamp (the dev cluster's third of
  the day), the dogfood graph re-ingested afterward.

## Test inventory

- `yeomna-pipeline` unit (7 new): three extract tests including a real
  docling conversion and every refusal kind, four notation tests
  including EC-7 and the unclosed block.
- `yeomna-store/tests/document_ingest.rs` (5 cluster-gated plus the
  ignored census): FR2 through FR7, EC-1, EC-2, EC-4, EC-7, the schema
  proof (open declared, kebab on both open partitions, shape still
  checked, structural closed), and promotion followed by fusion
  followed by the conforms link in one graph.
- The FR6 census runs with `cargo test -p yeomna-store --test
  document_ingest -- --ignored` when the corpus is present.

## The local review, and what it left standing

The pre-PR local review arrived as a workflow step after this PR opened,
so 015 took it mid-flight. Three findings were applied: the conforms
pass counted oversize and unreadable files instead of skipping them in
silence, the two `PipelineError` extraction variants got distinct
display strings, and the relation shape gained the test that makes its
four copies agree (the store test reads the regex out of the shipped
schema, proves both open partitions carry the same one, and has
Postgres evaluate it over the same sample the verb layer's unit test
pins). Three were recorded rather than applied, with reasons:

**A declaration that names a code node loses its payload if the
documents land first.** `write_declared` fuses onto an existing
non-document node and drops the block's `kind` and `tag`, and the code
ingest's upsert replaces a declared node's payload wholesale, so the
corpus's claim about a crate survives only when the code is ingested
first. On the census corpus this never fires (`nodes_fused` is zero,
because the corpus's node keys are crate names and claim slugs while
code keys are path-derived), which is why it is recorded rather than
rushed. The fix worth making is to stop storing the corpus's claim in a
payload another writer owns: a `declares` edge from the document to the
node carries the same information, needs no schema change now that
declared relations are open, and is order-independent. That is its own
small spec.

**The fuse pre-read, the placeholder's `overwrite: false`, and
promotion are three call-site policies over one sink upsert.** The rule
that a document writer must not clobber a code node lives in this
caller and nothing enforces it at the sink, so Phase 6's retire and
prune, and any later `node.insert`, inherit nothing. The general form
is a sink-level declare mode whose `ON CONFLICT` refuses to lower a
node's kind. Recorded for whoever writes the next document-kind writer.

**Re-declaring every file's blocks on every run is a repair loop.** It
is what makes an interrupted run self-heal, and it costs a read and an
upsert per declared node and edge on a run that changed nothing. The
alternative is ordering the writes so the content hash commits last and
gating it on zero rejections, which is cheaper and more fragile. The
loop stays until a corpus makes the cost visible.

## Riding items

- Widening the native extension set to the office formats is a later
  spec's call, with a corpus to test each against.
- EC-6 (documents deleted between ingests) waits on Phase 6's retire.
- The two placeholders in the census name what the WeaverTools docs
  reference and never declare, a finding for that repository.
