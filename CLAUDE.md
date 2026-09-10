# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

Documents: `README.md` is the Yeomna Charter PRD (draft v0.4, 2026-08-08).
Beneath it sit `docs/PRD-postgres-store.md`, `docs/PRD-pipeline-libraries.md`,
`docs/PRD-verb-layer.md`, `docs/PRD-embedder.md`, and the wire contract at
`docs/embedding-contract.md`, with specs at `docs/specs/NNN-slug/spec.md`
and review notes alongside each.

Code: a Cargo workspace, edition 2024, toolchain pinned by
`rust-toolchain.toml`, eleven crates, and two suites. **461 is what
`cargo test --workspace` reports passing** with the cluster and the
embedder up, which is the gate. It counts every Rust test the workspace
defines, because cluster-gated and service-gated tests pass by returning
early with a named skip when their dependency is absent: the number does
not move on a machine without one and it means less there, so a gate run
that matters is one where the skip lines are absent. The Python suite is
separate, `uv run pytest` in `services/embedder/`, and reports 59 with a
GPU and 54 without. **The Rust side of the
pipeline-libraries PRD is complete** (phases 1 through 5, specs 001 through
005): chunking, keys, batch, proto, embed, code, and pipeline are all lifted
and merged. **The store exists and holes-ledger H1 is filled** (specs 008
and 009, both merged 2026-08-13): `crates/yeomna-store` carries the schema,
applied to the dev cluster with eight claims as integration tests, and
`PgSink` implements the `IngestSink` trait
(`crates/yeomna-pipeline/src/sink.rs`), its only implementor, with the
five-call sequence as integration tests. All cluster-gated tests skip
without a cluster. Edge identity is ruled (R6, spec 009).

**H2 is complete in all seven phases and H8 is filled** (spec 021, PR #51,
merged 2026-09-10), which derived the CLI tree from the contract rather
than writing one beside it. **H4 is filled** (spec 022): the appliance embeds, `embed.text`
answers, and late chunking is wired for the first time in this code or
the reference's. `docs/PRD-embedder.md` owns that work and front-loads
its decisions as D1 through D11. Four things from it are worth knowing
before touching the embedding path:

- **The service pools and the wire carries chunk vectors** (D1). This is
  the opposite arm from the one `spikes/jina-late-loop` proved, and the
  reason is the transport the spike could not have weighed: one
  8,631-token document is 8631 by 2048 floats, roughly 212 MB once JSON
  has written each float as text, against 240 KB for the same document's
  29 pooled vectors.
- **`late_chunk_embeddings` is the oracle, not the producer.** It is not
  dead code. `POST /v1/tokens` exists only so a gated test can pool the
  token view in Rust and require the service's own pooling to match at
  cosine 0.9999. Do not give that operation a production caller.
- **`EmbeddingEndpoint` has no network variant and must not gain one**
  (D10). Charter 5.1 makes local embedding a requirement rather than a
  configuration default, and a requirement a config key can turn off is a
  preference.
- **The context ceiling is 16,384 tokens and it is a hardware fact.** The
  model advertises 32k and it does not fit on GPU 2: 8,631 tokens peaked
  at 11.48 GiB of 16. A document above the ceiling is refused, counted,
  and named, never truncated (D5), because truncation drops a document's
  tail out of vector search while leaving it in keyword search with
  nothing saying so.

**Hybrid query is built** (spec 023), so `query --hybrid` fuses keyword
and vector ranking by reciprocal rank fusion in one statement, which is
the claim the store PRD has carried since it was drafted. The query
vector is computed inside the verb at the task that pairs with the
corpus's, read off the rows, and is never accepted from the caller: a
`vector` field on the request would be a way to reach vector search
without calling `embed.text`, which is a side door around a verb. A graph
with no vectors, or with more than one cohort, is a refusal rather than a
quiet fall back to keyword ranking.

The history below is kept because the reasoning in it is still load
bearing.

**H2's construction, as it happened.** `docs/PRD-verb-layer.md` is
settled at v0.4 (all five review questions ruled) and Phase 1 of 7 is merged
(spec 010): `crates/yeomna-verbs` carries the closed verb contract (40 wire
names at Phase 1, 42 since R18 added the edge verbs in Phase 4), the
envelope, and the error taxonomy, with R4 closed so the wire names are
binding. The `audit_log.outcome` column landed with it under a column-scoped
grant, which is how attempt logging coexists with an append-only table. A
workspace lint keeps SQL inside `yeomna-store` and `yeomna-verbs`.

**Phase 2 is merged too** (spec 012, 2026-08-15) and the verb layer executes.
`yeomna-verbs` now carries the session, the exhaustive dispatch, the audit
write, and eleven verbs that read, which is why it is the second crate the
no-SQL lint admits. Every call leaves one audit row, reads included, and the
row commits before the verb runs, so an attempt that dies leaves a NULL
outcome rather than no trace. The schema version is a compiled-in constant
with no stored marker, because a marker exists to detect drift and drift is
not a state this appliance allows.

**Phase 3 is merged as well** (spec 013, 2026-08-17): the traversal verbs
per D7 with partition pruning proven at the verb level, graph.drop as the
first destructive verb, and the database lifecycle under the fourth role,
`yeomna_provision`, with kg-pattern databases stamped from
`yeomna_template` because pgvector is not a trusted extension. The session
serializes calls and retires itself if a role escalation cannot prove its
reset. M2 was measured 2026-09-09 (spec 016,
`docs/measurements/M2-recursive-cte-at-depth.md`): neither traversal
formulation blows up on the real graph, reachability saturates by depth 5,
D7 costs 12 ms at depth 100 with zero spill. Phase 4 followed, below.

**The hold ended 2026-09-09.** Resume verification passed: cluster up,
data intact, seal held, hugepages sufficient (4246 needed, measured the
ask-Postgres way), gate 310 green with cluster tests running. One
incident was found and fixed during the hold window: systemd-logind's
`RemoveIPC=yes` deleted the todd-owned Postgres shared memory segments
on a logout (2026-08-28), and the server limped a week before dying on
2026-09-04 with no data loss. The fix is
`/etc/systemd/logind.conf.d/50-remove-ipc.conf` setting `RemoveIPC=no`,
applied 2026-09-09. The deployment-era fix is a dedicated system user
for the unit, which is immune by category.

**The resumption driver is a semantic KG over the WeaverTools codebase**:
its code, documents, databases, services, and agents, and the edges
between them. The boundary is ruled (D9): Yeomna is an external RAG
appliance, never a part of the WeaverTools architecture, and its first
caller is Claude Code itself, using the graph as the RAG for building
these projects. **Phase 4 is merged** (spec 014, PR #36, 2026-09-09): seven
verbs including R18's `edge.assert` and `edge.retract`, the contract at
42 wire names, and the audit transaction proven by a crash-shaped test.
**Spec 015 followed the same day** (PR #39): extraction went native for
declarative formats through an `Extractor` trait, the socket client
unchanged behind it and the pinned `docling` converter beside it, and
documents joined the graph. The first census over the WeaverTools
corpus put 100 documents, 388 corpus-declared graph blocks (474 nodes,
649 edges in the corpus's own twelve relations), and all 492
`conforms:` headers into one graph of 5147 edges in 18 seconds, with
zero refusals and zero unresolved. Schema 1.2.0: the declared partition
speaks the source's own words, kebab included.

**The graph is reachable** (specs 017 and 018, PRs #43 and #45, filling
H10 and H6). `yeomna call` takes a verb request as JSON and prints the
envelope, which puts the whole 42-verb contract behind one command that
does not change as verbs land. Embedded mode links the verb layer, and
`--daemon` sends the same JSON to `yeomnad` over a Unix socket where
the caller is named from `SO_PEERCRED` and a uid this machine cannot
name is refused (D6). The appliance ships a config file, TOML at
`/etc/yeomna/yeomna.toml` with `YEOMNA_CONFIG` overriding the path, and
a file that exists and will not parse is refused rather than fallen
back from. `status` reports the session's actor, which is how a caller
sees who the appliance thinks it is, since no verb reads the audit log.

**Phase 6 is merged** (specs 019 and 020, PRs #47 and #49). Ingestion is a verb: `ingest`,
`codebase.ingest`, `codebase.drift` (which writes nothing and reports
what moved), `codebase.validate` (which finds the one invariant the
constraints cannot express, an edge whose endpoints live in another
graph), and the two destructive ones, `codebase.retire` and
`codebase.prune`. **T3's falsifying clause is answered**: nothing in
this product reaches the store without constructing a verb, and every
destructive call leaves an audit row naming its actor. What stays open
is capability rather than surface, and R22 asks separately whether the
record is rich enough. The charter's T3 paragraph and section 6 now say
what changed and what did not.

R23 was accepted by merging #49: `RetireRequest` carries a required
`path`, so retire sweeps only what the source truly lacks. **R22 stays
open**, and it is the third time the audit log's surfaces have come up:
the row records who asked and that it succeeded, not the substance of
what a destructive verb swept.

**Phase 7 finishes the verb layer** (spec 021), and H8 lands with it.
The CLI tree is born from the contract: every subcommand path is a wire
name with its dots as spaces (`yeomna graph neighbors`), and the request
is built by deserializing into the closed enum, so no verb has
hand-written argument code and a verb added to `Verb` is reachable the
same day. `--graph` means one thing and the contract decides whether it
is a request field or the session's scope. A table by default, `--json`
for the envelope, both entirely on stdout. `yeomna tools status` probes
the analyzers through the same resolver the ingest preflight uses, and
`tools install` places a binary the operator supplies rather than
fetching one (R24).

All seven of H2's phases are done. What remains on R21's order:
**the WeaverTools KG stand-up alone.** H4 filled in spec 022 and hybrid
query landed in spec 023, so the stand-up is the last of the ten and it
is last by Todd's order, since it touches their repository and their
deployment and wants a WeaverTools session in the loop.

**H3 is filled and this repository is a graph** (spec 011, merged
2026-08-15). `yeomna-pipeline` carries the codebase orchestrator beside the
document one: walk, analyze, hash-skip, chunk, embed, write, with every
language reaching the edge resolver built for it, chosen by the analyzer
that ran rather than by extension. The language-server pass (rust-analyzer,
gopls) is opt-in, gated per crate or module, and degrades to the structural
graph rather than failing. The dogfood graph `yeomna_self` holds 1976 nodes
and 3533 edges including 1325 `calls` after the 2026-09-09 re-ingests, the
corpus M2 wanted, and M2 was measured on it the same day (spec 016).

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

**The PR workflow.** Ruled 2026-08-10 as the per-crate lift workflow, and
rewritten 2026-09-09 as one execution-ordered list for every PR, lift or
build, when the local review and the merge rule arrived. Every PR gets:

1. A GitHub Issue documenting the work: for a lift the source paths, LOC,
   and coupling, and for a build the spec it builds.
2. A branch. For a lift the first commit is the verbatim move, diffable
   against the reference and the only surviving record of what the
   reference did, since the reference cannot run. For a build the first
   commits are the spec and the implementation.
3. **The local review.** Before the PR opens, the branch's diff against
   main goes through `/code-review <branch or PR> high`. Findings are
   fixed in follow-up commits on the branch or declined, and both are
   recorded in the spec's review notes (`docs/specs/NNN-slug/review-*.md`,
   or the PR description for a branch with no spec). A re-run after fixes
   scans the increment since the last pass. The point is throughput:
   CodeRabbit's allowance is throttled and every exchange costs a wait,
   so what it would catch is caught here first. PR #39 opened before this
   step existed and took the review mid-flight instead.
4. The PR opens ready for review, not as a draft, which triggers
   CodeRabbit.
5. CodeRabbit findings are addressed as **separate commits on the same
   PR**, never inside the move commit, each fixed and verified or declined
   with its reason on the PR. One push per exchange, fixes batched, since
   every push spends a throttled review.
6. **The merge rule.** CodeRabbit gets at most three exchanges, an
   exchange being a review with findings and the one push that answers
   it. When the review is clear within three exchanges, the workspace gate
   is green three times with cluster tests running, and the crate's
   end-to-end check confirms functionality, the PR merges without waiting
   on Todd. A fourth review that still finds a problem stops work for
   joint investigation, and so does, at any round, a finding that needs a
   ruling: a design decision, a spec contradiction beyond a recorded
   build-finding amendment, a repeated finding Claude keeps declining, or
   anything reaching the machine outside the branch and the dev cluster's
   documented rebuild. Docs-only commits that trigger incremental reviews
   do not count as exchanges.
7. After every merge: epic #21, the ledger, this file, the spec's review
   notes, and a hub report, which is how Todd catches up asynchronously
   while working elsewhere.

This amends the lift PRD's R1: defects found in transit are fixed in
follow-up commits on the branch, before or on the PR, but never inside the
move commit itself.

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
| Roles | `yeomna_owner` (DDL, superuser), `yeomna_app` (runtime), `yeomna_audit` (owns audit), `yeomna_provision` (CREATEDB NOLOGIN, reached by SET ROLE for database lifecycle, spec 013) |
| Database | `yeomna`, plus `yeomna_template` (the schema stamped and marked IS_TEMPLATE, what `database.create` copies for kg-pattern databases) |
| Memory | `shared_buffers=8GB`, `huge_pages=on`, `effective_cache_size=8GB`, `full_page_writes=off` (safe only on CoW storage) |

It runs as a systemd unit, `yeomna-postgres.service`, enabled at boot. Do not
start it with `pg_ctl` by hand.

```bash
sudo systemctl start yeomna-postgres
psql -h "$HOME/.local/share/yeomna/run" -p 5433 -U yeomna_owner -d yeomna
```

**The system `postgresql.service` is active again as of the 2026-08
hold**: it is WeaverTools' database now, on `127.0.0.1:5432`, run by the
`postgres` system user. Yeomna's seal is unaffected (nothing listens on
5433), but the old hazard is live again: a bare `psql` lands on 5432 and
the wrong project. Always connect with the socket path and port shown
above.

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

R15 (ruled 2026-09-09) keeps the pin through the PostgreSQL 19 cycle.
The SQL/PGQ property-graph feature was reverted from 19 before GA, so
19 offers this project nothing decisive, and D7's recursive CTEs never
assumed it. Revisit at PG 20 if SQL/PGQ lands there, and even then only
as an internal rewrite of traversal SQL behind unchanged verbs.

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
