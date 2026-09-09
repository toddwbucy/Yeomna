# The Holes Ledger

Status: v1.9, 2026-09-09. H1 and H3 retired, H2 three phases in.
The 2026-08-17 hold ended 2026-09-09 with the cluster verified and the
gate green (310, cluster tests running). The resumption has a driver:
**WeaverTools needs a knowledge graph** for its databases, services,
and agents, which makes it the verb layer's first customer and puts
Phase 4 and Phase 5 on the critical path. Spec 014 is drafted.
The hole-mapping step of the severance sequence
(PRD-pipeline-libraries v0.2). This is the map of everything Yeomna needs and
does not yet have, each hole named, owned, and sourced. The executable form
is `yeomna-cli`: 56 commands, every one a self-reporting hole with a census
test over them. That crate is **not on main**. It sits in draft PR #19 and
lands at H2 Phase 7, which rewrites its commands into daemon clients and
inverts the census, so the check runs nowhere until then. The authoritative
surface list in the meantime is spec 010's disposition table, which is on
main and which R4 made binding. As verbs land, holes become behavior, and
this document retires entry by entry.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually.

The standing rule for filling holes: **from Yeomna's own contracts** (the
`IngestSink` docs, the pinned JSON shapes, the golden keys, the captured CLI
surface, this ledger), never by consulting the closed reference.

---

## H1. The store. FILLED 2026-08-13

The largest hole and the reason the appliance exists. Postgres schema plus
the `IngestSink` implementation on the sealed cluster. Both halves are
merged and live: spec 008 (PR #23) put the schema on the cluster with the
seven claims as permanent tests, and spec 009 (PR #25) made `PgSink` the
trait's only implementor, retiring the spec 005 five-call deferral and
ruling edge identity (R6). The 28 CLI holes named below still wait on H2,
because the verbs are the surface and the store is what they reach. M3
(stale-delete window, concurrent ingest) is restated in the 009 review
notes and stays open.

| | |
|---|---|
| Owner | `docs/PRD-postgres-store.md`, phases 2 through 7 |
| Contract in hand | `crates/yeomna-pipeline/src/sink.rs` (two methods), byte-stable `chunk_doc`/`embedding_doc` JSON, the emitted types (`FileAnalysis`, `SymbolDocument`, `CrateEdge`, `TextChunk`), golden keys |
| CLI holes it fills | 28 (`db` tree, `status`, `orient`) jointly with H2 |
| Filled | the deferred five-call-sequence test (spec 005), edge identity (R6) |
| Still open | the M3 decisions, named not resolved |

## H2. The verb layer

The charter calls it the largest single block of work, and T3 stays false
until it exists. Dispatch, service, and daemon transport are rewritten, not
ported: the excluded reference code is not consulted.

| | |
|---|---|
| Owner | `docs/PRD-verb-layer.md`, settled v0.4 2026-08-13 |
| Contract in hand | the 32-verb inventory (store PRD), the captured CLI surface and envelope conventions (`yeomna-cli`), charter section 6 (audit log as shipped default, one audited entry point) |
| CLI holes it fills | the same 28, jointly with H1, plus `daemon` (H7) |
| Includes | Q3 renames of ArangoDB-vocabulary command names, the audit table wiring, peercred policy |
| Progress | **Phase 3 of 7 done.** Phase 3 (spec 013, PR #34): the graph answers, traversal pruning proven at the verb level, the first destructive verb, the kg pattern stamped from the template under `yeomna_provision` (R13/R13a), materialize refusing under R14, the session retiring itself on unproven role resets. Earlier: Phase 1 (spec 010, PR #27): the closed 40-verb contract, R4 closed, the audit outcome column. Phase 2 (spec 012, PR #32): eleven read verbs, the session, the exhaustive dispatch, and the audit write, with G1 as a permanent test over every implemented verb. The schema version is a compiled-in constant, ruled rather than stored |
| Still false | **T3.** The destructive paths are Phase 4 and Phase 6, so the charter's thesis stays false until they land and the CLI is repointed at Phase 7 |
| Next | **Phase 4, spec 014 drafted 2026-09-09**: the write verbs, the audit transaction (ok outcome durable if and only if the mutation is, crash-shaped test), the scoped `sql` verb under R17/R17a, and the R18 edge verbs the WeaverTools graph needs. Phase 5 follows immediately, since the daemon socket is what WeaverTools connects to |
| Waiting in Phase 7 | Draft PR #19, deliberately held 2026-08-15 rather than merged and fixed twice, since Phase 7 rewrites these commands into daemon clients anyway. It carries, and Phase 7 inherits: (1) the daemon `--mcp-*` flags, which the capture declares while its own record says they were dropped, plus a reference to a `--mcp-token-file` that does not exist, (2) `output.rs` printing table headers to stderr while rows go to stdout, so redirecting stdout loses the header, (3) awaits-distribution counts in the review notes that reach 56 by double-counting six commands, and (4) the R4 reconciliation: nine captured commands (`Create`, `Collections`, `Databases`, `CreateDatabase`, `Truncate`, `DropCollection`, `Export`, `CreateIndex`, `IndexStatus`) that spec 010 has since removed or absorbed, so the census counts 56 holes where roughly 40 become verbs. Trial-merged 2026-08-15 against main: conflicts are `Cargo.toml` and `Cargo.lock` only, and the crate builds, passes its census, and clears the no-SQL lint |

## H3. The ingest orchestrator. FILLED 2026-08-15

New construction wiring the crates to the sink: walk, analyze, two-pass
enrichment, hash-skip, chunk, embed, write. Merged as spec 011 (PR #30),
living in `yeomna-pipeline` beside the document flow. It replaced the
excluded `codebase_ingest.rs` without consulting it.

Every language reaches the resolver built for it, chosen by the analyzer
that ran rather than by file extension: syn plus rust-analyzer
for Rust, the rustpython AST for Python, libclang for C++, gopls for Go,
and tree-sitter for anything that fell back. The language-server pass is
opt-in, gated per crate or module, and degrades to the structural graph
rather than failing an ingest.

**This repository is now a graph**: 1620 nodes, 2650 edges including 861
`calls`, with call chains reaching the depth cap. That is the corpus M2
has always needed, and M2 stays open until the benchmark is run.

| | |
|---|---|
| Owner | store PRD Phases 4 and 7, spec 011 |
| Filled | R7 as R12 (`edge_basis` stays text), FR 2's enrichment protocol, the first dogfood ingest |
| CLI holes it fills | 9 (`codebase` tree, `ingest`), still waiting on H2 Phases 6 and 7 for their verbs |
| Still open | M2's benchmark, M3's concurrency window, Python call edges wired but unexercised |

## H4. The embedder backend

The reference's Python loader was ruled out (boilerplate around a loader the
SPU replaces). The weaver-spu embedder operation is the primary path.

| | |
|---|---|
| Owner | joint contract document with WeaverTools, then the SPU embedder in the WeaverTools repo |
| Contract in hand | the spike evidence (`spikes/jina-late-loop`): token-level 2048-d hidden states plus tokenizer offsets, client-pools proven at cosine 0.999999, boundary metadata as token ranges converting to bytes at the edge |
| CLI holes it fills | 6 (`embed` tree) |
| Also fills | late-chunking wiring (specified in the reference, never wired), the PE-API successor contract with a Yeomna-native name, retirement of the client's TCP default |
| Constraint | 32k-context late-chunking-capable model, GPU 2 |
| Measured 2026-09-09 | weaver-spu has no embedder operation (the Python embedder retired at its PR-1.J and nothing replaced it), so this contract means adding an embed directive to the SPU wire protocol, not pointing at an existing one |
| Harvest pointers (from docling-rag, read not adopted, R19) | the deterministic hash embedder as a test double for cluster-anywhere embedding tests, and RRF hybrid fusion as reference when the `hybrid` query flag lands |

## H5. The extraction backend

**R19 ruled 2026-09-09 (revising R5): the declarative half goes native.**
R5's revisit trigger fired when the WeaverTools corpus arrived (51k
lines of load-bearing Markdown that a semantic KG cannot do without),
and spec 015 adopts the `docling` converter crate, pinned, behind an
extraction trait, PDF feature off. Markdown proves it now and the
office formats ride along untested until a corpus needs one. **R19b
closed PR #17 unmerged the same day** (branch kept as the severance
record): upstream validates against Python docling continuously, so
the wrapper added nothing the ledger had not already harvested. The
R5 spike (captions, formulas, on our documents) is still owed before
the PDF path ships, run against the Rust engine directly, with
pip-installed docling as the side-by-side if one is wanted. Spec 015
also carries the `conforms:` doc-to-code resolver, the linkage that
makes the WeaverTools graph semantic.

The original R5 reasoning, kept for the record:

The reasoning, and the correction that came with it:

- **The ledger named the wrong crate.** `docling-rs` on crates.io
  (0.1.2, February 2026, 174 downloads) is a third-party HTTP client for
  Docling Serve, so building against it would have kept the Python
  service in the loop rather than removing it. The engine is `docling`
  and `docling-core` (1.12.0, released 2026-08-15, from the official
  `docling-project/docling.rs`). Any future work names those.
- **The engine is young and moving fast.** A dependency releasing on the
  day we look at it is churn we would carry, and low adoption means we
  find the bugs rather than inherit the fixes.
- **Nothing needs extraction yet.** The dogfood corpus is code. No
  document ingest is blocked, so the switch buys no capability today
  and costs a port plus a review.
- **A first-run model download of roughly 700MB from a GitHub release**
  is an appliance-packaging problem under charter section 5, and it is
  the same problem on either side of this choice.

**Revisit when** a document corpus needs ingesting, which is
when H5 starts blocking something, or when the engine's release cadence
settles. At that point the measurement to run is named below rather
than rediscovered.

| | |
|---|---|
| Owner | its own spec, when the revisit trigger fires |
| Contract in hand | the `yeomna.extraction` wire protocol (proto plus Rust client on main), the Python service's behavior |
| The surface a replacement must cover | six things, measured 2026-08-15 from `docling_backend.py`: `do_table_structure`, `do_ocr`, and `do_cell_matching` on the PDF pipeline, `export_to_markdown`, tables with captions, equations, figures with captions, and page count. LaTeX has its own native backend and the PyMuPDF fallback is not docling, so neither is at risk |
| Unverified, and what a spike would settle | whether table and figure captions survive, whether formula extraction behaves (ours carries a fallback because `doc.equations` was unreliable, and the Rust side puts formulas behind enrichment that is off by default), and whether the engine's own byte-for-byte parity claim holds on our documents |
| Riding this hole | the PR #17 server-architecture findings retired with the server (R19b): they described a Python service that will not ship. The R5 spike remains the live item |

### The scope this hole has, ruled 2026-08-15

Two things were tangled together here and are now separated.

**Repository connectors are not Yeomna.** The original shape of the idea
was a service that reaches arXiv, and later PubMed, JSTOR, Wikipedia, or
the licensed sources a law firm already pays for, fetches on an
authorized account, and feeds a knowledge graph. That is a **separate
application**: a network port on one side, Yeomna's Unix socket on the
other. Charter 5.2 already assigns it there, since anything needing the
wire lives on the far side of the socket and owns that concern itself.
Its outbound search queries and its stored credentials are its
compliance story to tell, not this appliance's, and neither appears in
this ledger.

**What Yeomna owes that world is one general PDF path.** Every one of
those repositories hands over a PDF, either generated from LaTeX or
scanned at high quality and OCRed. A source parser answers that for
exactly one repository, and only because arXiv is unusual in publishing
source at all.

So the LaTeX and arXiv backend is a **drop candidate**, not inherited
scope to be ported. It handles `.tex` and `.tar.gz` source packages to
recover exact equation markup, citations, and sections, which is a real
capability and the wrong generalization: it serves one source instead of
all of them.

**Before it goes, one thing needs proving.** The Python backend hedges
its PDF equation extraction, trying `doc.equations` and then scanning
`doc.texts` for formula entries when that comes back empty. Somebody
wrote that fallback because the first path failed on real documents. So
"the PDF is enough" is the right bet and an untested one, and it is the
same spike R5 already names, on the same documents.

**The formats to cover** are the buyer's, per charter section 4: a law
office, a medical practice, an accounting firm. That means DOCX, XLSX,
PPTX, email, HTML, RTF, legacy Office, and scanned images needing OCR,
alongside PDF. Two notes make this cheaper than it sounds. Docling's
declarative formats skip the model download entirely, so the office
half needs no GPU and no staged models, unlike PDF. And the router's
final branch already sends unknown extensions to docling, so a DOCX may
land today undeclared rather than unsupported, which a test would
settle in an afternoon.

## H6. The daemon and transports

The socket server the CLI's `daemon` command awaits. Rewritten against the
verb layer, carrying the peercred trust boundary.

| | |
|---|---|
| Owner | the verb-layer PRD (H2) |
| Contract in hand | charter 5.2 (default-deny, openings named), the captured daemon CLI surface (MCP flags dropped, parked) |

## H7. The schema manager

Declarative YAML bootstrap (`schema apply`). The reference's
`schema_apply.rs` is excluded as store-coupled.

| | |
|---|---|
| Owner | verb-layer era, small |
| CLI holes it fills | 1 |

## H8. The analyzer toolchain manager

`tools status` and `tools install`. Notably the shallowest hole:
`yeomna-code` already carries `resolve_and_probe` and the managed-tools-dir
logic (`YEOMNA_TOOLS_DIR`), so this is mostly wiring.

| | |
|---|---|
| Owner | any convenient spec, candidate first hole to fill |
| CLI holes it fills | 2 |

## H9. The graph-embed era

Structural embeddings: GraphSAGE per charter 8.3, the training service, the
graph loader and export. The excluded `graph/`, `training.rs`,
`hades-prefetch`, and the Python training service are not consulted: the
capability is rebuilt against the Postgres graph when its era arrives.

| | |
|---|---|
| Owner | its own future PRD |
| CLI holes it fills | 3 (`graph-embed` tree) |
| Feeds | the two replay axes of charter 8.3 (differential provenance, model-version replay) |

## H10. The config layer

The appliance ships a config file, not a scatter of env vars (the
`postgresql.conf` precedent). Every service currently carries env surfaces
under Yeomna names as interim.

| | |
|---|---|
| Owner | R3 ruled 2026-08-14, then small work in each consumer |
| Product surface today | one variable, `YEOMNA_TOOLS_DIR` (`yeomna-code`, `managed_tools_dir`). The embedder endpoint, CLI `--db` resolution, and daemon socket paths were listed here before they existed and still do not |
| Not in scope | test and development knobs, which stay environment variables: `YEOMNA_TEST_DB`, `YEOMNA_CUDA_FIXTURE`, `YEOMNA_RA_BIN`. `HOME` and `CARGO_MANIFEST_DIR` are facts of the OS and the build, not configuration |
| Lands with | its first real consumer, the daemon (H2 Phase 5) or the CLI (PR #19). Building a config crate before one exists would be vocabulary ahead of its consumer |

## H11. Review follow-up ledgers

Recorded during lifts, riding in the review notes, none blocking:

- **004 (`yeomna-code`):** diagnostics-aware tiering (degraded cpp parses
  report Semantic), the parse-once refactor, `find_item_start` blank-line
  spans (identity-adjacent, needs its own scrutiny), cpp_edges canonicalize
  caching, CLANG_LOCK analyzer-thread design, go.work grouping, preflight
  timeout, and the CUDA kernel-launch capability gap on this box (needs a
  compilation database or clang crate feature work).
- **005 (`yeomna-pipeline`):** the five-call-sequence test. Landed with H1
  (spec 009), store side.
- **006 (extraction):** everything riding H5.
- **003 follow-up:** the embed client integration tests (type and config
  portions) belong in `yeomna-embed`. **Done 2026-08-14**
  (`tests/config_and_types.rs`): the shipped defaults for both clients,
  `ExtractOptions::all()` keeping OCR opt-in, and a guard that neither
  client consults the environment. Pinned deliberately as the values
  H10's config file has to reproduce when R3's ruling is implemented.
- **002 (`yeomna-batch`):** per-item checkpointing O(N squared), revisit
  when a measured ingest shows the cost.

## Open rulings

| # | Ruling | State |
|---|---|---|
| R1 | Q2 graph isolation | **Ruled 2026-08-12: graph_id column**, one table set, partition-by-graph as measured graduation. Recorded in the store PRD |
| R2 | `full_page_writes` on ZFS | **Ruled 2026-08-12: off**, CoW invariant stated in the conf. H1 unblocked |
| R3 | Config file versus env (H10) | **Ruled 2026-08-14: config file.** The appliance ships one, on the `postgresql.conf` precedent. Test and development knobs stay environment variables, since they configure a harness rather than a product. Measured the same day: the live product surface is one variable |
| R4 | Q3 verb naming | **Closed 2026-08-14 (spec 010): renamed.** The 40 wire names are binding, `Db` prefixes and document-store vocabulary gone |
| R13/R13a | Database lifecycle role and the template stamp | **Ruled 2026-08-17 (spec 013)**: `yeomna_provision` reached by SET ROLE, kg databases stamped from `yeomna_template` since pgvector is not trusted |
| R14 | `graph.materialize` | **Ruled 2026-08-17**: keeps its R4-bound name, refuses until a consumer defines materialization |
| R5 | Extraction direction | **Ruled 2026-08-15: not now.** The Rust engine is real and official but young and fast-moving, and no document ingest is blocked, so the Python service stays interim and PR #17 stays in draft. Revisit when a corpus needs ingesting. The ledger's old `docling-rs` name pointed at a third-party Docling Serve client, not the engine |
| R6 | Edge identity | **Ruled 2026-08-13 (spec 009): an edge is (graph_id, src_id, dst_id, relation, basis)**, analyzer and status and payload are attributes. `edges_identity` unique index, claim 8 |
| R7 | `edge_basis` Rust mapping | **Ruled 2026-08-15 as R12 (spec 011): text at the boundary**, cast in SQL, no second enum to drift from the DDL |
| R15 | PostgreSQL major version | **Ruled 2026-09-09: stay pinned at 18.x.** PG 19 is still in beta, the repos carry 18.6, and the feature that would have mattered, SQL/PGQ property graphs, was reverted from 19 on 2026-09-07 for design issues. D7's recursive CTEs never depended on it. Revisit at PG 20 GA (expected late 2027) if SQL/PGQ returns, as an internal traversal rewrite behind unchanged verbs |
| R16-R18 | Session ownership, the `sql` verb's role and detection, the edge verbs | **Agreed 2026-09-09 and built (spec 014)**. R16: the client lives inside the call lock. R17/R17a: `sql` runs as the provision role on a per-call connection, kg pattern detected structurally. R18 amends the contract (40 to 42 verbs, `edge.assert` and `edge.retract`), exposed by the first customer: a deployment graph is mostly edges and the contract could not write one. The build moved the relation CHECK into the partitions (SCHEMA_VERSION 1.1.0) and let the store PRD's inherited cascade rule delete and purge, both recorded in the 014 review notes |
| R19 | Native extraction (revises R5) | **Agreed 2026-09-09, specced (015)**: adopt the `docling` converter crate only, pinned, PDF feature off, behind an extraction trait, proven on the WeaverTools Markdown corpus. The R5 spike still gates the PDF flip (its reference role passed to upstream and pip docling under R19b, which closed #17 the same day). `docling-rag` declined: its defaults are a second store, a remote LLM, and a foreign chunker, three charter violations before configuration. What changed since R5: the format migration is complete and byte-for-byte validated upstream, and declarative formats need no ML assets |
| R19b | PR #17 closed unmerged | **Ruled 2026-09-09**: the Python extraction service closes without merging, branch kept as the severance record. R19 retired its declarative role, upstream's continuous validation against Python docling serves the reference role better than our wrapper would, and its unique findings (the six-item surface, the equation fallback) were already harvested into H5. The last living port ends and HADES-Burn is fully closed. If a PDF corpus arrives before the Rust engine's PDF pipeline proves out, the closed branch is the resurrection point |
| R19a | Declared vocabulary is the source's own | **Ruled 2026-09-09 (v2, spec 015)** after the dig found the WeaverTools docs declare their own graph: 387 fenced graph blocks, 473 nodes, twelve edge relations. The `edges_declared` CHECK opens to identifier shape like `edges_asserted` (sources speak their own words), `edges_structural` keeps the closed list (our analyzers, our vocabulary), corpus-declared nodes land as kind `document` with block kind and tag in payload (the methodology disposition landing as ruled), SCHEMA_VERSION 1.2.0. Declined: enumerating any corpus's ontology into the DDL, which would make every future customer a schema migration. Todd's framing, now binding: a semantic KG is installation-specific and grown through use, so the substrate is fixed and the vocabulary is the operator's |

## The fill order, as the dependencies read

1. R1 and R2 rule, then the store schema spec (H1 begins). Done, spec 008.
2. The sink implementation (done, spec 009), the ingest orchestrator
   (done, spec 011), first end-to-end code ingest and the dogfood of this
   repo (done 2026-08-15). H1 and H3 both retired.
3. The verb-layer PRD (H2, with H6 and H7), turning CLI holes into behavior.
4. The embedder contract and SPU (H4) with late chunking wired.
5. The extraction backend (H5) when document corpora arrive.
6. H9 and the replay axes, then methodology returns as corpus.
