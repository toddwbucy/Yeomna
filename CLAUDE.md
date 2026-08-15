# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

Documents: `README.md` is the Yeomna Charter PRD (draft v0.4, 2026-08-08).
Beneath it sit `docs/PRD-postgres-store.md`, `docs/PRD-pipeline-libraries.md`,
and specs at `docs/specs/NNN-slug/spec.md` with review notes alongside.

Code: a Cargo workspace, edition 2024, toolchain pinned by
`rust-toolchain.toml`, nine crates, 282 tests. **The Rust side of the
pipeline-libraries PRD is complete** (phases 1 through 5, specs 001 through
005): chunking, keys, batch, proto, embed, code, and pipeline are all lifted
and merged. **The store exists and holes-ledger H1 is filled** (specs 008
and 009, both merged 2026-08-13): `crates/yeomna-store` carries the schema,
applied to the dev cluster with eight claims as integration tests, and
`PgSink` implements the `IngestSink` trait
(`crates/yeomna-pipeline/src/sink.rs`), its only implementor, with the
five-call sequence as integration tests. All cluster-gated tests skip
without a cluster. Edge identity is ruled (R6, spec 009).

**H2, the verb layer, is under construction.** `docs/PRD-verb-layer.md` is
settled at v0.4 (all five review questions ruled) and Phase 1 of 7 is merged
(spec 010): `crates/yeomna-verbs` carries the closed 40-verb contract, the
envelope, and the error taxonomy, with R4 closed so the wire names are
binding. The `audit_log.outcome` column landed with it under a column-scoped
grant, which is how attempt logging coexists with an append-only table. A
workspace lint keeps SQL inside `yeomna-store` and `yeomna-verbs`. Next is
Phase 2 (read verbs).

**H3 is filled and this repository is a graph** (spec 011, merged
2026-08-15). `yeomna-pipeline` carries the codebase orchestrator beside the
document one: walk, analyze, hash-skip, chunk, embed, write, with every
language reaching the edge resolver built for it, chosen by the analyzer
that ran rather than by extension. The language-server pass (rust-analyzer,
gopls) is opt-in, gated per crate or module, and degrades to the structural
graph rather than failing. The dogfood graph `yeomna_self` holds 1620 nodes
and 2650 edges including 861 `calls`, which is the corpus M2 wanted. M2
stays open until the benchmark is run and reported.

The pipeline-libraries PRD's own Phase 6, the Python services, waits on the
SPU and config rulings. Note that phase numbers are per PRD and do not
correspond across them. Commands are
`cargo build`, `cargo test`, `cargo clippy --all-targets`,
`cargo fmt --check`, all from the repository root. Per charter section 13,
the string `yeomna` is what belongs in crate metadata.

## How work is done here

PRD, then spec, then build to spec. This is the WeaverTools discipline
(`~/olympus/git-repos/WeaverTools`) and it carries over unchanged. Do not write
code ahead of a spec, and do not write a spec ahead of a PRD.

The house format, for matching rather than reinventing:

- **PRD** at `docs/PRD-<topic>.md`. Revision History, Executive Summary,
  Background and Context, User Stories, Goals with an explicit Non-Goals list,
  phased Feature Specifications, Technical Architecture including a Resolved
  Design Decisions block, Testing Strategy, Risk Assessment, Timeline.
- **Spec** per phase, numbered and slugged, one directory each. Overview, Task
  Scope with an Out of Scope list, Files to Modify, Files to Reference, Patterns
  to Follow, Functional Requirements, Edge Cases, Implementation Notes as DO and
  DON'T, Success Criteria, QA Acceptance Criteria. WeaverTools keeps these at
  `.auto-claude/specs/NNN-slug/spec.md`, which is tied to its tooling. **This
  repo uses `docs/specs/NNN-slug/spec.md`**, alongside the PRDs. First one is
  `docs/specs/001-workspace-and-chunking/spec.md`.

**No graph vocabulary ahead of a graph** (ruled 2026-08-10). No primitive,
relation, or basis enums, no ontology crate, no graph modeling of this repo's
own documents or code while the store does not exist. The lifted code already
carries the strings it writes. Vocabulary gets defined when the thing that needs
it exists: basis with the store schema, everything else post-hoc through
dogfooding. The pull toward pre-modeling is constant and each instance looks
reasonable alone. Decline them all.

One PRD spawns many numbered specs. WeaverTools ran one PRD out to specs 024
through 031 across phases 1, 2a, 2b, 2c, and 3.

**Migrating code from the reference is an orderly, reviewed, documented act.**
Nothing gets bulk-copied. Each piece that comes over is reviewed and documented
as it lands, against the spec that called for it. The measurements in the port
posture section say the move is mechanically small. They do not license skipping
the review, and a low coupling count is a reason the review is cheap rather than
a reason to skip it.

**Per-crate lift workflow, ruled 2026-08-10.** Every crate brought over gets:

1. A GitHub Issue documenting the move (source paths, LOC, coupling, review
   notes).
2. A branch and a **draft PR**. The first commit is the verbatim move,
   diffable against the reference. Todd takes the PR out of draft manually,
   which triggers CodeRabbit review.
3. CodeRabbit findings are addressed as **separate commits on the same PR**,
   never squashed into the move commit. The move commit is the only surviving
   record of what the reference did, since the reference cannot run.
4. Once all CodeRabbit comments are addressed, e2e testing on the crate
   confirms functionality.
5. Then, and only then, merge to main.

This amends the lift PRD's R1: defects found in transit are fixed on the PR in
follow-up commits rather than deferred, but never inside the move commit
itself.

**Current focus: PRDs and specs for the Postgres end.** The rest waits until that
is done. Charter section 14 names the first target, which is a technical spec
covering the schema, the verb inventory, and the ingestion operation set.

`docs/PRD-postgres-store.md` is that first PRD, drafted at v0.1. Ruled during
drafting: the **verb inventory belongs in the store PRD** (the 32 kept commands
are the store's surface contract), while the **verb layer implementation is a
separate later PRD**. Do not pull the implementation into store scope. The PRD
carries four open questions of its own (Q1 basis for the 118 unattributed edges,
Q2 graph isolation mechanism, Q3 verb naming, Q4 pointing at M3) and those need
rulings before the phases they block can be specced.

## The development cluster

A Yeomna-owned Postgres cluster exists on this machine, built 2026-08-10. It is
**not** the system instance, which listens on `127.0.0.1:5432` and therefore
violates charter section 5. Do not develop against the system instance.

| Property | Value |
|---|---|
| Version | PostgreSQL 18.4, pgvector 0.8.6 |
| ZFS dataset | `dbpool/yeomna`, inherits recordsize 8K, lz4, atime off |
| Data dir | `~/.local/share/yeomna/pgdata` |
| Socket dir | `~/.local/share/yeomna/run`, mode 0700, port 5433 |
| Logs | `~/.local/share/yeomna/log` |
| Roles | `yeomna_owner` (DDL), `yeomna_app` (runtime), `yeomna_audit` (owns audit) |
| Database | `yeomna` |
| Memory | `shared_buffers=8GB`, `huge_pages=on`, `effective_cache_size=8GB`, `full_page_writes=off` (safe only on CoW storage) |

It runs as a systemd unit, `yeomna-postgres.service`, enabled at boot. Do not
start it with `pg_ctl` by hand.

```bash
sudo systemctl start yeomna-postgres
psql -h "$HOME/.local/share/yeomna/run" -p 5433 -U yeomna_owner -d yeomna
```

**The system `postgresql.service` is disabled**, because it bound
`127.0.0.1:5432` and a bare `psql` would land there instead of here. The package
stays installed, since this cluster runs its `/usr/bin/postgres`.

The seal is held at three layers, all verified 2026-08-10:

| Layer | Mechanism |
|---|---|
| `postgresql.conf` | `listen_addresses = ''` |
| `pg_hba.conf` | both `host` rules are `reject` |
| systemd unit | `RestrictAddressFamilies=AF_UNIX` |

The third is the one that matters most, because it means an edit to
`listen_addresses` cannot reopen the network on its own. The property is
confirmed applied to the unit. A behavioral test, attempting a TCP bind and
watching the kernel refuse, has not been run.

Auth is peer with an ident map from the `todd` account, which mirrors the
appliance model where filesystem permission on the socket is the gate. Verify
the seal with `ss -ltn | grep 543`, which should return nothing.

The unit also carries `RequiresMountsFor` on the dataset, so a boot that has not
mounted `dbpool/yeomna` refuses to start rather than initialising onto the bare
mountpoint, and `Restart=no`, because a restart loop cannot conjure huge pages
that were never allocated.

### Package pinning

`postgresql`, `postgresql-libs`, and `pgvector` are in `IgnorePkg` in
`/etc/pacman.conf`. The ground does not move during development. A PostgreSQL
major upgrade would need `pg_upgrade` against this data directory, and pgvector
is built per major, so holding one without the other breaks the extension load.

Updates are a deployment concern and get revisited when development is done.
This is also charter section 5's tradeoff arriving early: stock Postgres means
upstream's binary and upstream's cadence, and at development time the pin is how
that cadence gets refused.

The dataset is dedicated so it can be snapshotted independently, which is what
charter section 8.3 needs for copy-on-write checkpoint versioning. It carries
`primarycache=metadata` locally, so ARC does not double-cache what
`shared_buffers` already holds.

### Memory sizing, and the frame it follows

Ruled 2026-08-12: **this is an agentic database app, and optimization targets
traditional database performance, never RAM-cache maximization.** The corpus
is RAM-resident at modest cache sizes (the reference's largest graph measured
single-digit GB), an agent's query costs planner plus index descent plus a
socket round trip, and the RAM on this box belongs to the models. The
original 64GB `shared_buffers` was WeaverTools-era bleed-over, where RAM is
the workload. Resized to 8GB, with `effective_cache_size` matching it because
`primarycache=metadata` means nothing beyond `shared_buffers` caches data
pages. `full_page_writes` is off (R2), safe only while pgdata lives on
copy-on-write storage, stated in the conf comment.

Memory settings live in `postgresql.conf`, not in `ALTER SYSTEM`, because an
appliance ships a config file rather than a runtime override. `huge_pages =
on` stays strict (fail loud), and at this size the reservation is trivial.

Two rules, learned the hard way on 2026-08-10, still binding:

1. **Ask Postgres, do not compute.** `postgres -D <datadir> -C
   shared_memory_size_in_huge_pages` prints the exact count (server stopped).
   Hand arithmetic once produced a count 152 pages short and a failed
   service. Over-reserving is safe, under-reserving is the failure.
2. **`HugePages_Free` is not what is available.** `HugePages_Rsvd` is counted
   inside Free. Usable is `Free - Rsvd`.

Current allocation is `vm.nr_hugepages = 5000` in
`/etc/sysctl.d/98-hugepages.conf`, comfortably above the roughly 4.9k the 8GB
setting commits. The pool is global and shared with the (default-sized,
disabled) system instance.

Undecided, raised and not ruled: `full_page_writes = off`, which is safe on ZFS
because copy-on-write never tears a page. It is currently **on**, the Postgres
default.

## Editorial rules for every document in this repo

Stated in the charter's header and binding on the charter and its descendants:

- ASCII only.
- No em-dashes.
- No semicolons.
- Never the words *genuinely*, *honestly*, or *actually*.

These are mechanical and worth checking before handing back prose. They apply to
documents written here, not to quoted external material.

## What the charter is, and how to treat it

The charter is the founding artifact. It fixes the context a project is built
inside. Two rules from its header govern how to read it against the reference
implementation, HADES-Burn:

- Where the charter and the reference disagree about **what the product should
  be**, the charter wins.
- Where they disagree about **what something costs**, the reference wins.

The charter cites the reference by measured fact (line counts, edge counts,
timings). Treat those numbers as findings already made. Re-derive one only when
a decision turns on it, and see the next section for where to look.

Section 12 is a decision record whose purpose is to keep settled questions
closed. Section 13.2 does the same for rejected names. Do not reopen an entry
there without new evidence, and do not re-argue ArangoDB, Neo4j, Kuzu,
SurrealDB, Memgraph, FalkorDB, or SQLite as store candidates.

## The reference implementation, HADES-Burn

It lives at `~/olympus/HADES-Burn` (worktrees at `~/olympus/HADES-Burn.worktrees`).
It is a **separate git repository**, not a submodule and not part of this one.
Read it freely. Do not edit it, do not commit there, and do not run its build as
though it tested anything here. Its own `CLAUDE.md` and `AGENTS.md` govern any
work done inside it, including that production ArangoDB databases are
read-only and that `bident_burn` is the writable project database.

It is a Rust workspace, edition 2024, roughly 50k lines across five crates:
`hades-cli` (binary `hades`), `hades-core` (the library that matters here),
`hades-proto`, `hades-prefetch`, `hades-frontend`. Its commands are
`cargo build`, `cargo test`, `cargo clippy`, run from its own root.

Pointers for the harvest list in charter section 11:

| What | Where |
|---|---|
| Bounded operation vocabulary, the closest thing to a verb layer | `crates/hades-core/src/dispatch.rs` (about 7.9k lines, `DaemonCommand` plus exhaustive dispatch) |
| Access tiers, session policy, the trust boundary above transport | `crates/hades-core/src/service.rs` |
| Unix-socket daemon and framing | `crates/hades-core/src/daemon_client.rs`, `crates/hades-cli/src/commands/daemon.rs` |
| Chunking, including late chunking | `crates/hades-core/src/chunking/` |
| Code analysis (rust-analyzer, syn, rustpython, libclang, tree-sitter) | `crates/hades-core/src/code/` |
| Batch ingestion pipeline | `crates/hades-core/src/batch/`, `crates/hades-core/src/pipeline/` |
| Deterministic key derivation | `crates/hades-core/src/db/keys.rs` |
| Embedder service contract (PE-API v1) | `docs/persephone-embedding-api.md`, `crates/hades-core/src/persephone/` |
| ArangoDB client, left behind entire | `crates/hades-core/src/db/` |
| The dead ANN module, 444 lines, confirmed | `crates/hades-core/src/db/vector.rs` |

The six commands that reach the store without constructing a verb, per section 6,
are `crates/hades-cli/src/commands/codebase_{ingest,retire,prune,drift,validate}.rs`
and `graph_embed_update.rs`. That directory is the map of the work the verb layer
has to cover.

Also worth reading for intent: `service.rs` describes transports as owning "only
framing and connection lifecycle," with "the Unix-socket listener today, a
network endpoint tomorrow." The store boundary was drawn deliberately, and the
coupling measurements below are what that intention looks like measured.

## Port posture: brownfield code, greenfield data

This is a port, not a fresh build. What does not come over is the data. T4 and
section 12 rule out migration entirely, the existing graphs are rebuildable test
data, and there is no migrate command to write.

**The reference is not running, and cannot be without a project.** Verified
2026-08-10: the ArangoDB *data* survives on `dbpool/arangodb`, 50.3G referenced
with snapshots back to October 2025, but ArangoDB the software is not installed,
there is no `arangodb3.service`, HADES-Burn has never been built (no `target/`
directory), and the embedder and extractor services are down. The corpus exists
and nothing can read it.

Two consequences, and neither is fatal:

- **The reference's source is the specification. Its behavior is not
  observable.** Differential testing is a capability that would have to be
  bought by installing the engine the charter rejected, building 50k lines, and
  standing up a GPU embedder. Do not write plans that assume it.
- **The charter's measured graph facts are historical findings.** 18,506
  documents, 19,024 edges, 6,129 attributed calls edges, 118 unattributed. Treat
  them as findings already made, per the charter's own rule, and do not plan to
  re-derive one without budgeting the resurrection.

**The port scope is the store and close to nothing else. That is by design, and
it measures.** Store coupling across the reference is about 1,028 lines out of
roughly 53.5k Rust plus Python, near 2 percent, and it is concentrated rather
than spread.

### The pipeline is the product, and it is store-free

Ingestion and encoding are about two thirds of the code that survives the port
intact, and they carry almost no coupling:

| Component | LOC | Real store coupling |
|---|---|---|
| `code/` analysis | 9,294 | 0 (7 doc-comment mentions) |
| `chunking/` incl. late chunking | 733 | 0 |
| `batch/` resumable, fault-isolated | 1,220 | 0 |
| `persephone/` embedder and extractor clients | 1,067 | 0 |
| Python `services/` (Docling, LaTeX, PyMuPDF fallback, Jina V4) | 3,320 | 0 across 32 files |
| `pipeline/orchestrator.rs` | 647 | 4 |
| `codebase_ingest.rs` | 3,873 | 67 |

About 20.1k LOC, roughly 71 truly coupled lines, all of them in two files.
The extraction and embedding services are separate processes behind a socket and
an HTTP contract and have never referenced the store at all. They do not get
ported. They get pointed at a new sink.

The orchestrator seam is four points: the `ArangoPool` import, an error variant,
the `db` struct field, and the constructor parameter. That is the whole boundary
between 12k lines of ingest machinery and the database.

### Build order

1. Lift the store-free mass as pure libraries with no store dependency, per
   section 14's first slice. Roughly 12.3k LOC moves with no edits. Introduce a
   sink abstraction at the orchestrator's four coupling points.
2. Write the schema and methodology as a document, not code. This is the
   technical spec section 14 calls for, and it is where basis-as-partition-key,
   the diff log, and the edge tables get settled.
3. Build the graph in Postgres and repoint the sink. `codebase_ingest.rs` and its
   67 coupled lines across about 17 blocks are the real porting cost here.
4. Then the queries and the verb layer, against a schema that exists.

Step 4 comes last on purpose. The verb layer cannot be designed against tables
that have not been defined.

5. Dogfood. This repository and the reference become the first corpus, ingested
   by Yeomna into a knowledge graph. Methodology reenters here, as content.

**The lift is what settles the schema, and step 2 depends on step 1 for that
reason.** The emitted types are the specification for the tables. Reading them
on 2026-08-10 produced three things the ontology document did not: edge
provenance in the reference is a free-form JSON convention rather than a typed
field, which is the root cause of the 118 unattributed edges (store PRD Q1),
edge endpoints arrive as collection-qualified strings so the sink owns text to
id resolution (D1), and chunk-to-symbol linkage is orchestrator work rather than
chunker output. Do not write tables ahead of the types that fill them.

Step 5 is not a victory lap. It is where the tool gets used on itself, and it is
the first corpus honest enough to test against. Note what it feeds without
closing: M2 asks for cycle behavior and row growth at depth on a real code graph
where calls edges cycle, and dogfooding produces exactly that graph. It supplies
the benchmark corpus. It does not supply the answer, and M2 stays open until the
benchmark is run and reported. M1 wants a real firm's document set, which this is
not, so dogfooding informs it and cannot settle it.

### Parity is feature parity, and it is a design intent

Copy solutions, not structure. **Parity means the kept features exist, not that
outputs match byte for byte, and it carries no percentage.** Do not write a
parity number into any document, do not build a differential harness to chase
one, and do not benchmark Yeomna against the reference.

The reason is that the reference was not displaced for being slow or wrong. It
was displaced because ArangoDB Community caps at 100 GiB and bars commercial
production use, so the buyer lands on enterprise pricing. The problem is a
license, and a license is not fixed by matching behavior more closely. Charter
section 10 already retired the performance comparison, on the grounds that the
remembered ArangoDB baseline was never a graph-engine result.

Per layer, what that means concretely:

- **Analysis, chunking, batch, encoders:** the code moves. Behavior is preserved
  by construction rather than by measurement, which is why the lift forbids
  refactoring in transit.
- **Ingestion and retrieval:** the 32 kept verbs exist and do the job their
  contract names. Same feature, freely better implementation.
- **Store layer and verb layer:** no structural parity at all, by design.

Measurement still applies where the question is whether Postgres suffices in
absolute terms, which is M1 through M4. Those are not comparisons.

The denominator is not the whole reference. Of 55 `DaemonCommand` variants, the
charter drops or forbids 23, leaving **32**:

- 18 `Task*` variants. Task management is left behind, section 11.
- `DbAql`. Section 6 forbids a raw query surface, so porting it ports the hole.
- 4 `Smell*` and `LinkCodeSmell`. Methodology, ruled below.

A high-fidelity port would also faithfully reproduce the 25-of-57 verb-layer
bypass. Do not. On that surface the behavior is kept and the path is replaced.

One thing the port scope does not cover: closing the verb layer is new
construction, not a port, and section 6 calls it the largest single block of work
in the build. It sits alongside the store swap rather than inside it.

### Naming: the mythology stays behind

Ruled 2026-08-11. Getting off the old mythological naming scheme is a project
goal, and lifts apply it at the cheapest moment, which is before anything
deploys. Persephone (the reference's ML-boundary brand, covering the embedder
and extractor services and their protocols) does not enter Yeomna: the wire
package became `yeomna.extraction` at the Phase 3 lift, the crates are
`yeomna-proto` and `yeomna-embed`, and PE-API's successor contract gets a
Yeomna-native name at birth. The test for what to rename: brands and package
names are mythology and change, while service, rpc, message, and field names
are engineering and stay. Contrast with the keys contract, which is frozen
because golden values guard stored-data idempotency. A name is frozen only
while something deployed speaks it.

### Three things, kept apart

Every piece of the reference sorts into one of three buckets, and the sorting is
what keeps the port scoped:

1. **Methodology.** A way of working. Belongs in a document. Never in the
   binary.
2. **The application.** Ingest, encode, retrieve, serve. Ports nearly intact,
   per the table above.
3. **The mechanical store need.** A graph-semantic-relational backend in
   Postgres. Built fresh.

A methodology encoded as a tool is documentation that leaked into the binary, and
it does not get ported. It gets written down.

`hades smell` is the worked example and the ruling is closed: it is compliance
checking, meaning check code for smells, verify `CS-NN` claims, emit a report.
That is a practice, bucket 1. It drops from the port.

Dropped is not deleted. Methodology returns after the store works, as corpus
rather than as code: smells and claims become documents and edges in the graph,
reached through the verb layer, not `SmellCheck` variants in dispatch. The
reference's own collection vocabulary already names the shape, since alongside
files, symbols, and chunks it carries axioms and specs. Build the instrument
first, then write with it.

Two reasons it is worth stating rather than leaving to judgment. It fails the
section 2.2 inherited-versus-invented test, since what is invented here is the
sealing, the verb layer, and the provenance model, and a smell checker is none of
those. And its name argues for keeping it under false pretenses: `smell` is
compliance of code against claims, a developer practice, while this product's
compliance is a firm against its regulator. The words collide and the referents
do not. Do not let the homonym pull it back into scope.

## Architecture the code must obey

The product is a sealed appliance: ingestion, storage, retrieval, and serving all
happen inside one box carrying its own Postgres instance. The store is commodity.
The sealing is the invention.

Constraints that any design or code must hold:

- **One store, one engine.** Postgres supplies documents (JSONB + GIN),
  embeddings (pgvector), keyword retrieval (tsvector + GIN, ranked with
  ts_rank_cd), graph (ordinary tables reached by recursive CTEs), and
  transactions across all of them. No second store of any kind, including caches
  and external search.
- **Unix socket only.** The store binds no TCP listener. The backend is
  default-deny on the network permanently. Anything needing the wire lives on the
  far side of the socket and is a front-end concern (this is where MCP would
  live, and MCP is parked).
- **The verb layer is the only surface.** Nobody writes SQL, JSONB path
  expressions, traversals, or search calls. Human and agent call the same verbs
  and are logged the same way. This line does not exist yet and building it is
  the largest block of work: in the reference, six CLI commands (codebase
  ingest, retire, prune, drift, validate, graph-embed update) reach the store
  directly, and roughly 25 of 57 query sites sit outside the verb boundary,
  including the destructive ones.
- **Basis is derived at ingest and written explicitly.** Never infer provenance
  from edge type. Edges partition by LIST on basis (declared, structural,
  asserted). Status (ratified, pending) is an ordinary mutable column, not a
  partition key.
- **The diff log is the source of truth for history.** Append-only, per node.
  The head row is materialized convenience. If head and log disagree, the log
  wins.
- **The graph is a rebuildable index.** It is derived from source by ingestion,
  so losing it costs a re-ingest. There is no migration tooling, no migrate
  command, and no data-fidelity requirement. Do not write any.
- **Nothing leaves the box.** No telemetry, no phone-home, no remote embedding,
  no default remote backup destination. A remote model is not the default. The
  three real boundaries where data can leave are named in section 5.1.
- **Inherit, do not author.** If a capability is standard database work, take it
  from Postgres (access segmentation via the customer's directory, row-level
  enforcement, clustering, retention via cascade and cron). Checkpoint
  versioning drops to the filesystem as copy-on-write snapshots. What is
  invented is the sealing, the verb layer, and the provenance model.

## Open items, and their status

Sections 10 and 14 hold what is unsettled: FTS relevance on a real corpus (M1),
recursive CTE behavior at depth including cycles (M2), ingest isolation under
concurrent write and serve (M3), and erasure against an append-only history
(M4, unsolved). Appendix A lists three v0.4 changes folded pending a
strike-or-keep ruling. Do not present any of these as resolved, and do not quietly resolve
one in passing while doing other work.

The theses in section 3 are labeled falsifiable on purpose. T3 in particular is
false today until the verb layer covers the destructive paths.
