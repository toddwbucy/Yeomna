# The Holes Ledger

Status: v2.1, 2026-09-09. H1 and H3 retired, H2 four phases merged
(Phase 4 merged as PR #36, the first merge under the three-exchange
rule). The 2026-08-17 hold ended 2026-09-09
with the cluster verified and the gate green (327, cluster tests
running). The resumption has a driver: **a semantic KG over the WeaverTools
codebase**, its databases, services, agents, and documents. The
boundary is ruled (D9): Yeomna is an external RAG appliance, and its
first caller is **Claude Code itself**, using the graph as the RAG
for building these projects, with weaver agents and buyers behind it.
That puts Phases 4 through 6, the client surface (epic #37, the
`yeomna call` client above all), and the document graph (spec 015) on
the critical path. **The severance is complete**: R19b
and R20 closed the last two held drafts unmerged (branches kept as
records), so nothing of the reference remains in flight.
The hole-mapping step of the severance sequence
(PRD-pipeline-libraries v0.2). This is the map of everything Yeomna needs and
does not yet have, each hole named, owned, and sourced. The authoritative
surface list is spec 010's disposition table, on main, R4-binding,
now grown to 42 wire names by R18. Completeness checking moved from
the retired capture's runtime census to the closed enum itself: a
dispatch or CLI that exhaustive-matches `Verb` cannot omit a verb and
still compile. As verbs land, holes become behavior, and this
document retires entry by entry.

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
| Contract in hand | the 42-wire-name contract (spec 010's disposition table plus R18), the envelope conventions pinned in `yeomna-verbs`, charter section 6 (audit log as shipped default, one audited entry point) |
| CLI holes it fills | the same 28, jointly with H1, plus `daemon` (H7) |
| Includes | Q3 renames of ArangoDB-vocabulary command names, the audit table wiring, peercred policy |
| Progress | **Phase 4 of 7 done.** Phase 4 (spec 014, PR #36, issue #35, merged 2026-09-09): seven verbs including R18's `edge.assert` and `edge.retract`, the audit transaction proven by a crash-shaped test, the scoped `sql` verb under R17/R17a, R16's client inside the call lock. Phase 3 (spec 013, PR #34): the graph answers, traversal pruning proven at the verb level, the first destructive verb, the kg pattern stamped from the template under `yeomna_provision` (R13/R13a), materialize refusing under R14, the session retiring itself on unproven role resets. Earlier: Phase 1 (spec 010, PR #27): the closed 40-verb contract, R4 closed, the audit outcome column. Phase 2 (spec 012, PR #32): eleven read verbs, the session, the exhaustive dispatch, and the audit write, with G1 as a permanent test over every implemented verb. The schema version is a compiled-in constant, ruled rather than stored |
| Still false | **T3.** The destructive paths are Phase 4 and Phase 6, so the charter's thesis stays false until they land and the CLI is repointed at Phase 7 |
| Next | **R21's order, #36 and 015 merged, M2 measured (spec 016)**: `yeomna call`, the daemon with its config file, the WeaverTools KG stand-up, Phase 6 in two halves (T3 goes true with retire and prune), the CLI tree with H8, the embedder, hybrid query |
| The client surface | **Re-scoped 2026-09-09 under R20 into epic #37.** PR #19 (the capture) closed unmerged, branch kept: its contract role passed to spec 010's table, its census role to compile-time completeness from the closed enum, its code to reference material. The epic parts out the daemon (Phase 5), `yeomna call` (the agent surface, embedded mode first), the contract-born per-verb CLI tree (Phase 7), H8's tools commands, and H10's config file, with the capture's five findings carried as do-not-reproduce items |

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

**This repository is now a graph**: 1976 nodes and 3533 edges after the
2026-09-09 re-ingests, 1325 of them `calls`. That is the corpus M2 always
needed, and **M2 was measured on it 2026-09-09** (spec 016,
`docs/measurements/M2-recursive-cte-at-depth.md`): neither traversal
formulation blows up, reachability saturates by depth 5, the D7 walk
costs 12 ms at depth 100 with zero spill.

| | |
|---|---|
| Owner | store PRD Phases 4 and 7, spec 011 |
| Filled | R7 as R12 (`edge_basis` stays text), FR 2's enrichment protocol, the first dogfood ingest |
| CLI holes it fills | 9 (`codebase` tree, `ingest`), still waiting on H2 Phases 6 and 7 for their verbs |
| Still open | M3's concurrency window, Python call edges wired but unexercised (M2 measured 2026-09-09, reopens on a corpus whose hubs do not saturate by depth 20, or on any spill at all in a D7 walk) |

## H4. The embedder backend

The reference's Python loader was ruled out (boilerplate around a loader
that is not ours to keep). **The primary path is a Yeomna-native in-box
embedder service (D9, ruled 2026-09-09).** The SPU's future embedder work
is agent-internal and stays in WeaverTools.

| | |
|---|---|
| Owner | its own Yeomna PRD (D9 ruled 2026-09-09): a native in-box embedder service, GPU 2, Jina-class late-chunking model, built on our own spike evidence. **The boundary ruling behind it:** Yeomna is an external RAG appliance and WeaverTools is never its component supplier. The SPU will gain embedder operations someday, but those are agent-internal cognition, a different job from appliance-internal indexing |
| Contract in hand | the spike evidence (`spikes/jina-late-loop`): token-level 2048-d hidden states plus tokenizer offsets, client-pools proven at cosine 0.999999, boundary metadata as token ranges converting to bytes at the edge |
| CLI holes it fills | 6 (`embed` tree) |
| Also fills | late-chunking wiring (specified in the reference, never wired), the PE-API successor contract with a Yeomna-native name, retirement of the client's TCP default |
| Constraint | 32k-context late-chunking-capable model, GPU 2 |
| Measured 2026-09-09 | weaver-spu has no embedder operation (the Python embedder retired at its PR-1.J and nothing replaced it), which is part of why D9 re-ruled this hole native: the SPU path was new construction in the wrong repo |
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

**Spec 015 built 2026-09-09 (issue #38), in review.** The first census
over the WeaverTools corpus: 100 documents, 1518 chunks, 388 graph
blocks with zero refusals, 474 declared nodes, 649 declared edges
carrying all twelve of the corpus's relations, and all 492 `conforms:`
headers resolved to declared claims with zero unresolved. One graph,
5147 edges, 18 seconds. The docling pin held against a crate that
published five versions on the build day.

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

## H6. The daemon and transports. FILLED 2026-09-09

The socket server, built as spec 018 (PRD Phase 5). Length-prefixed JSON
frames, one verb request in and one envelope out, with the calling uid
read from the kernel at accept and written into every audit row for that
connection. D6 holds: a uid this machine cannot name is refused rather
than admitted under a synthetic one. One `Session` per connection, so
the session's serialization and its retirement retire one client rather
than the appliance. The unit ships at `deploy/yeomna-daemon.service`
with `RestrictAddressFamilies=AF_UNIX`, which makes charter 5.2's
default-deny a property of the service rather than a promise in a
config file.

Transport and nothing else: the crate holds no verb decision and emits
no SQL, in deliberate contrast to the reference's 7.9k-line dispatch
file. `yeomna call --daemon` sends the same JSON the embedded mode
takes, so the transport is a deployment choice and the contract does
not notice.

| | |
|---|---|
| Owner | the verb-layer PRD (H2), built as spec 018 |
| Contract in hand | charter 5.2 (default-deny, openings named), R21 D5 (the frames and the unit), D6 (an unresolvable peer uid is refused) |
| Filled | the socket, the codec, peercred as the actor, the unit, and the CLI's framed transport |

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
logic (`YEOMNA_TOOLS_DIR`), so this is mostly wiring. **Unblocked by
R20**: it was waiting on the captured CLI and now lands with the
contract-born CLI in epic #37.

| | |
|---|---|
| Owner | epic #37, its own small spec |
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

## H10. The config layer. FILLED 2026-09-09

The appliance ships a config file, not a scatter of env vars (the
`postgresql.conf` precedent). Spec 017 wrote it: TOML at
`/etc/yeomna/yeomna.toml`, `YEOMNA_CONFIG` naming another path for
development, a documented default for every key so a machine without
the file still runs, and a file that exists and will not parse refused
rather than fallen back from, since falling back would run the
appliance against a store the operator did not name.
`docs/yeomna.toml.example` is the shipped shape. What it says is where
the store is, never what a verb does.

| | |
|---|---|
| Owner | R3 ruled 2026-08-14, written by spec 017 |
| Product surface today | one variable, `YEOMNA_TOOLS_DIR` (`yeomna-code`, `managed_tools_dir`). The embedder endpoint, CLI `--db` resolution, and daemon socket paths were listed here before they existed and still do not |
| Not in scope | test and development knobs, which stay environment variables: `YEOMNA_TEST_DB`, `YEOMNA_CUDA_FIXTURE`, `YEOMNA_RA_BIN`. `HOME` and `CARGO_MANIFEST_DIR` are facts of the OS and the build, not configuration |
| Landed with | `yeomna call` (spec 017), its first real consumer, which is the condition this row set. The daemon inherits the file at Phase 5 rather than inventing a second one. Still outside it, and still environment variables, are the test and development knobs above |

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
| R21 | The ten-PR plan decisions | **Ruled 2026-09-09**, front-loading every decision the next ten PRs need so the loop runs without mid-flight rulings. D1: long-running verbs block (synchronous, generous timeout, the session already serializes, no job vocabulary in a closed contract). D2: `drift` re-walks and hash-compares, reports changed/missing/new, writes nothing. D3: `retire` removes nodes whose sources are gone, `prune` removes orphans, both naming what they swept in audit args (T3's sentence). D4: config is TOML at `/etc/yeomna/yeomna.toml`, dev override `YEOMNA_CONFIG`. D5: daemon socket beside the PG socket, 0600, 4-byte length-prefixed JSON frames, 16MB max, one session per connection, unit mirroring yeomna-postgres with the RemoveIPC lesson. D6 (as corrected by Todd): **an unresolvable peercred uid is refused, not admitted under a synthetic name**, the refusal logged with the raw uid. D7: one `weavertools` database, one graph holding code, docs, and the asserted deployment layer, because the linkage across them is the point. D8: M2 is report-only, measurement not threshold. D10: CLI wire dots become spaces. D9, ruled after the boundary discussion: **H4 is Yeomna-native**, an in-box embedder service under its own PRD. Yeomna is an external RAG appliance, WeaverTools is never its component supplier, and the callers arrive in order: Claude Code sessions first (the graph as the RAG for building these projects), weaver agents later over the socket like any client, buyers eventually |
| R20 | PR #19 closed unmerged, the client surface re-scoped | **Ruled 2026-09-09**: the capture's contract role was superseded by its own product (spec 010's binding disposition table), its census role is replaced by compile-time completeness from the closed enum (a CLI that exhaustive-matches `Verb` cannot omit a verb and build), and its lift role thinned to reference material since the request structs are now the arg shapes. Branch kept. Epic #37 parts out the client surface (daemon, `yeomna call`, the contract-born CLI tree, H8's tools commands, H10's config file) with the five #19 findings carried as do-not-reproduce items. H8 is unblocked |
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
