# The Holes Ledger

Status: v1.0, 2026-08-12. The hole-mapping step of the severance sequence
(PRD-pipeline-libraries v0.2). This is the map of everything Yeomna needs and
does not yet have, each hole named, owned, and sourced. The executable form
is `yeomna-cli`: 56 commands, every one a self-reporting hole whose census is
machine-checked on every test run. As verbs land, holes become behavior, and
this document retires entry by entry.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually.

The standing rule for filling holes: **from Yeomna's own contracts** (the
`IngestSink` docs, the pinned JSON shapes, the golden keys, the captured CLI
surface, this ledger), never by consulting the closed reference.

---

## H1. The store

The largest hole and the reason the appliance exists. Postgres schema plus
the `IngestSink` implementation on the sealed cluster.

| | |
|---|---|
| Owner | `docs/PRD-postgres-store.md`, phases 2 through 7 |
| Contract in hand | `crates/yeomna-pipeline/src/sink.rs` (two methods), byte-stable `chunk_doc`/`embedding_doc` JSON, the emitted types (`FileAnalysis`, `SymbolDocument`, `CrateEdge`, `TextChunk`), golden keys |
| CLI holes it fills | 28 (`db` tree, `status`, `orient`) jointly with H2 |
| Also fills | the deferred five-call-sequence test (spec 005), the stale-delete atomicity decision (M3, named in the orchestrator comment) |
| Blocked by | nothing. R1 and R2 both ruled 2026-08-12 |

## H2. The verb layer

The charter calls it the largest single block of work, and T3 stays false
until it exists. Dispatch, service, and daemon transport are rewritten, not
ported: the excluded reference code is not consulted.

| | |
|---|---|
| Owner | a verb-layer PRD, not yet written |
| Contract in hand | the 32-verb inventory (store PRD), the captured CLI surface and envelope conventions (`yeomna-cli`), charter section 6 (audit log as shipped default, one audited entry point) |
| CLI holes it fills | the same 28, jointly with H1, plus `daemon` (H7) |
| Includes | Q3 renames of ArangoDB-vocabulary command names, the audit table wiring, peercred policy |

## H3. The ingest orchestrator

New construction wiring the seven crates to the sink: walk, analyze,
two-pass enrichment, hash-skip, chunk, embed, write. Replaces the excluded
`codebase_ingest.rs` and the document ingest flow. A few hundred lines
against tested parts, not a port.

| | |
|---|---|
| Owner | store PRD Phase 7 era, possibly its own spec |
| Contract in hand | `yeomna-code` (enrichment and `symbol_hash` live there), `yeomna-batch` (resume semantics), `yeomna-pipeline` (the flow), `IngestSink` |
| CLI holes it fills | 9 (`codebase` tree, `ingest`, `extract` jointly with H6) |

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

## H5. The extraction backend

Ruled 2026-08-12: moves to docling-rs for real multithreading and memory
management. The ported Python service (PR #17, held in draft) is behavioral
reference and interim option only.

| | |
|---|---|
| Owner | its own spec when the direction call settles |
| Contract in hand | the `yeomna.extraction` wire protocol (proto plus Rust client on main), the Python service's behavior (backend routing, OCR flags, idle unload, metadata shape) |
| Riding this hole | the skipped server-architecture findings from the PR #17 review, service-level tests, the one-line `convert` stopgap if the Python path must run interim |

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
| Owner | a ruling first, then small work in each consumer |
| Touches | extractor env surface, embedder endpoint default, CLI `--db` resolution semantics, daemon socket paths |

## H11. Review follow-up ledgers

Recorded during lifts, riding in the review notes, none blocking:

- **004 (`yeomna-code`):** diagnostics-aware tiering (degraded cpp parses
  report Semantic), the parse-once refactor, `find_item_start` blank-line
  spans (identity-adjacent, needs its own scrutiny), cpp_edges canonicalize
  caching, CLANG_LOCK analyzer-thread design, go.work grouping, preflight
  timeout, and the CUDA kernel-launch capability gap on this box (needs a
  compilation database or clang crate feature work).
- **005 (`yeomna-pipeline`):** the five-call-sequence test, lands with H1.
- **006 (extraction):** everything riding H5.
- **003 follow-up:** the embed client integration tests (type and config
  portions) belong in `yeomna-embed`.
- **002 (`yeomna-batch`):** per-item checkpointing O(N squared), revisit
  when a measured ingest shows the cost.

## Open rulings

| # | Ruling | State |
|---|---|---|
| R1 | Q2 graph isolation | **Ruled 2026-08-12: graph_id column**, one table set, partition-by-graph as measured graduation. Recorded in the store PRD |
| R2 | `full_page_writes` on ZFS | **Ruled 2026-08-12: off**, CoW invariant stated in the conf. H1 unblocked |
| R3 | Config file versus env (H10) | Not ruled |
| R4 | Q3 verb naming | One option left standing (rename), lands with the verb spec |
| R5 | PR #17 extraction direction | Held in draft pending docling-rs shape |

## The fill order, as the dependencies read

1. R1 and R2 rule, then the store schema spec (H1 begins).
2. The sink implementation, the ingest orchestrator (H3), first end-to-end
   code ingest, dogfooding this repo.
3. The verb-layer PRD (H2, with H6 and H7), turning CLI holes into behavior.
4. The embedder contract and SPU (H4) with late chunking wired.
5. The extraction backend (H5) when document corpora arrive.
6. H9 and the replay axes, then methodology returns as corpus.
