# PRD: Postgres Store

Status: draft v0.1, 2026-08-09. Sits beneath the Yeomna Charter PRD (README.md,
draft v0.4). Where this document and the charter disagree, the charter wins.
Where this document and HADES-Burn disagree about cost, the reference wins.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. SQL and code blocks keep
the syntax their language requires.

---

## Revision History

| Version | Date | Change |
|---|---|---|
| 0.1 | 2026-08-09 | First draft. Schema, sealing, verb inventory, ingestion operation set. |

---

## Executive Summary

This PRD covers the mechanical store need: a graph-semantic-relational backend in
Postgres, sealed to a Unix socket, that the existing HADES-Burn ingestion and
encoding pipeline can write into and the verb layer can read out of.

Three facts set the scope.

The pipeline does not need porting. About 20.1k LOC of analysis, chunking,
batching, and encoding carries roughly 71 store-coupled lines, and the extraction
and embedding services have never referenced a database at all. They need a sink,
not a rewrite.

The schema is not being invented. HADES-Burn ships a 731-line ontology
specification at `docs/codebase-graph-ontology.md` with five semantic primitives,
four vertex collections, four edge collections, six indices, and seventeen
validation invariants. This PRD translates that ontology into Postgres and
applies the charter's provenance model to it.

The data does not migrate. T4 rules it out, the existing graphs are rebuildable
test data, and there is no migrate command in scope.

What is invented here is narrow: the sealing, the provenance model expressed as
partitioned tables plus an append-only diff log, and the verb surface over both.

---

## Background and Context

### The Problem

HADES-Burn works and its store cannot ship. ArangoDB Community caps at 100 GiB
aggregate and bars commercial production use, so the intended buyer lands on
enterprise pricing (charter section 12). It is also an unfamiliar name in a sale
where familiarity is the product (charter section 2.1).

Underneath the licensing problem sits a capability problem. The reference has no
engine full-text search. Its hybrid path computes term coverage in Rust with no
term frequency, no IDF, and no length normalization. Its ANN module is 444 lines
with zero callers, and the live search path brute-forces cosine in Rust and
refuses past 100,000 embeddings.

### Why Now

The pipeline is finished and the store is the only thing standing between it and
a shippable appliance. The coupling has been measured at about 2 percent of the
tree, so the window where this is a contained job is open now.

### The Opportunity

Postgres supplies five capabilities in one engine, and two of them are upgrades
over the reference rather than ports of it:

| Capability | Mechanism | Versus reference |
|---|---|---|
| Documents | JSONB, GIN-indexed | Parity |
| Embeddings | pgvector, halfvec, HNSW | New capability |
| Keyword retrieval | tsvector, GIN, ts_rank_cd | New capability |
| Graph | ordinary tables, recursive CTEs | Parity |
| Transactions | across all of the above | Upgrade |

The reference's own ontology document carries an open question, Q1, asking
whether to add a full-text index over chunk text and noting the trade-off is
index maintenance against hybrid search happening in the query language instead
of a Rust-side reranker. **Q1 is answered by the substrate change.** tsvector
plus pgvector in one statement is what Postgres does.

### Development Strategy

PRD, then spec, then build to spec. Code migrated from the reference is reviewed
and documented as it lands, per repository policy. The low coupling count makes
the review cheap, not skippable.

---

## User Stories

### Compliance Officer

Asks what stores the client files. Receives one answer, "Postgres," and a
statement that the store binds no network port. Both close the question rather
than opening one.

### Firm Principal

Buys a box. It installs, runs, and gets backed up by the IT support already
retained, using tooling that support already recognizes.

### Agent

Calls a verb. Receives JSON. Never writes SQL, never writes a path expression,
never hand-builds a traversal, and is logged the same way a person is.

### Ingest Operator

Points the appliance at a repository or a document set. The pipeline runs
resumable and fault-isolated, and every destructive operation it performs leaves
a verb-layer record. Today `codebase retire` leaves none.

---

## Goals

### Primary Goals

1. **G1. One engine, five capabilities.** Documents, embeddings, keyword
   retrieval, graph, and transactions in a single Postgres instance. No second
   store of any kind, including caches and external search.
2. **G2. Sealed to a socket.** No TCP listener. The store is unreachable across
   the network even from the same machine.
3. **G3. Provenance that cannot be bypassed.** Basis derived at ingest from the
   analyzer and written explicitly, never inferred from edge type. Edges
   partitioned by LIST on basis. Partition pruning survives the recursive term.
4. **G4. History as the source of truth.** Append-only diff log per node. The
   head row is materialized convenience. If head and log disagree, the log wins.
5. **G5. A sink the pipeline can write into.** The orchestrator's four coupling
   points accept a Postgres target and the 20.1k LOC upstream of them does not
   change.
6. **G6. A named verb inventory** covering the 32 kept commands plus the
   ingestion operations that have no verb today.

### Non-Goals

1. **Migration tooling.** Per T4. No migrate command, no data-fidelity
   requirement, no importer from ArangoDB. Existing graphs stay on the reference.
2. **The verb layer implementation.** This PRD names the inventory and fixes the
   contracts. Building the audited surface, closing the 25-of-57 bypass, and
   wiring the audit log are separate work and the charter calls that the largest
   single block in the build.
3. **A raw query surface.** `DbAql` has no Postgres counterpart by design.
   Section 6 forbids it. Porting it ports the hole.
4. **Task management and smell.** Methodology and out of scope, per repository
   policy. They return after dogfooding, as corpus rather than as code.
5. **A custom-compiled Postgres.** Stock binaries and upstream's CVE feed.
   Shipping a build pipeline owned forever is the trap the charter names.
6. **Clustering and replication.** Node-to-node work that does not exist until a
   second node. Nothing to author and nothing to scope.
7. **MCP and any network endpoint.** Front-end concerns on the far side of the
   socket. Parked.
8. **Resolving M1, M2, or M4.** Named in Risk Assessment with their graduation
   triggers, and left open.

---

## Feature Specifications

### Phase 1: Instance and Sealing

The appliance ships its own Postgres instance assembled from stock parts.

**Socket only.** `listen_addresses = ''` and a configured
`unix_socket_directories`. There is no port to secure because there is no port.

**Extension versions.** pgvector 0.7 or later, for halfvec. Shipping the instance
also removes the `CREATE EXTENSION` superuser step from a customer who has no DBA.

**Correction to the charter's framing of the dimension ceiling, measured
2026-08-10 on the development cluster.** The charter attributes the failure to
pgvector 0.6.0 and implies that a newer pgvector retires the constraint. Tested
against **pgvector 0.8.6 on PostgreSQL 18.4**:

| Type at 2048 dims | HNSW index | Result |
|---|---|---|
| `halfvec(2048)` | `halfvec_cosine_ops` | Creates |
| `vector(2048)` | `vector_cosine_ops` | `ERROR: column cannot have more than 2000 dimensions for hnsw index` |

The 2000-dimension ceiling is a property of the `vector` type and it persists in
current pgvector. Version 0.7 or later is necessary and **not sufficient**. The
`halfvec` type is mandatory, not an optimization, and switching a column to
`vector` at any future point silently removes the ability to index it. Treat this
as a constraint on the schema and not as advice.

**Roles.** Three at minimum, separated so the appliance operator does not own
the audit log:

| Role | Holds |
|---|---|
| `yeomna_owner` | Schema DDL. Not used at runtime. |
| `yeomna_app` | Runtime read and write on data tables. INSERT only on audit. |
| `yeomna_audit` | Owns the audit table. UPDATE and DELETE revoked from everyone. |

**Audit log integrity as a shipped default.** Append-only, UPDATE and DELETE
revoked, separate owning role. Every mechanism is stock Postgres and none is on
by default, which is why the charter commits to turning them on here rather than
leaving them to deployment. Appendix A item 1 of the charter holds this pending a
strike-or-keep ruling, so it is built as specified and flagged, not treated as
settled.

**Access segmentation is inherited, not authored.** Postgres integrates with the
customer's Active Directory or LDAP. Authentication and policy stay the
customer's. Row-level enforcement is ordinary RLS.

### Phase 2: Nodes, Chunks, Embeddings

The reference's ontology defines five semantic primitives and states that every
vertex is exactly one of them: `file`, `module`, `type`, `callable`, `value`. It
splits them across two collections only because `file` carries different fields.
In Postgres they are one table with a `kind` column and a JSONB payload.

```sql
CREATE TABLE nodes (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    graph_id    bigint NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    natural_key text   NOT NULL,
    kind        node_kind NOT NULL,
    payload     jsonb  NOT NULL,
    UNIQUE (graph_id, natural_key)
);

CREATE INDEX ON nodes USING gin (payload);
CREATE INDEX ON nodes (graph_id, kind);
```

**Surrogate primary key, natural key preserved.** The reference's `_key` values
are deterministic and human-readable, such as `src_lib_rs__Config__new__a1b2c3d4`
from `keys::symbol_key`. Determinism is what makes re-ingest idempotent and it is
kept as a UNIQUE constraint. Edges reference `bigint` instead, because an edge row
carries two endpoints and a recursive CTE walks them repeatedly. This is a
resolved decision, recorded in Technical Architecture with its reasoning.

**Chunks and embeddings** stay separate tables, since they are orphan documents
in the reference ontology and not graph vertices.

```sql
CREATE TABLE chunks (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id     bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    chunk_index integer NOT NULL,
    text        text   NOT NULL,
    span        int4range NOT NULL,
    symbol_ids  bigint[] NOT NULL DEFAULT '{}',
    tsv         tsvector GENERATED ALWAYS AS (to_tsvector('english', text)) STORED,
    UNIQUE (node_id, chunk_index)
);

CREATE INDEX ON chunks USING gin (tsv);
CREATE INDEX ON chunks USING gin (symbol_ids);

CREATE TABLE embeddings (
    chunk_id   bigint PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    vec        halfvec(2048) NOT NULL,
    model      text   NOT NULL,
    model_hash text   NOT NULL
);

CREATE INDEX ON embeddings USING hnsw (vec halfvec_cosine_ops);
```

**The deletion cascade is inherited.** Section 7.3 of the reference ontology
specifies a six-step manual cascade and notes that a `codebase purge` command
"should" perform it. Foreign keys with ON DELETE CASCADE do it instead. This is
the charter's inherit-do-not-author test applied, and it deletes a command
from the build.

**The `symbols[*]` array index becomes a GIN index on `bigint[]`.** Same reverse
lookup, same one-index cost.

### Phase 3: Edges, Basis, and Traversal

The reference uses four edge collections and states that the collection name IS
the relation, with no type discriminator on the document. The charter overrides
this: edges live in one logical table partitioned by LIST on basis. Relation
becomes a column and basis becomes the partition key.

```sql
CREATE TYPE edge_basis AS ENUM ('declared', 'structural', 'asserted');

CREATE TABLE edges (
    id        bigint GENERATED ALWAYS AS IDENTITY,
    graph_id  bigint NOT NULL,
    src_id    bigint NOT NULL,
    dst_id    bigint NOT NULL,
    relation  edge_relation NOT NULL,
    basis     edge_basis NOT NULL,
    status    edge_status NOT NULL DEFAULT 'ratified',
    analyzer  text,
    payload   jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (id, basis)
) PARTITION BY LIST (basis);

CREATE TABLE edges_declared   PARTITION OF edges FOR VALUES IN ('declared');
CREATE TABLE edges_structural PARTITION OF edges FOR VALUES IN ('structural');
CREATE TABLE edges_asserted   PARTITION OF edges FOR VALUES IN ('asserted');
```

**Status is a column, not a partition key.** Ratified and pending change in
place with no row movement.

**Basis changes are plain UPDATEs with automatic row movement**, measured at
2.25 ms in the charter for the realistic case, plus a diff log entry so the prior
basis stays recoverable.

**Partition pruning survives the recursive term.** Re-verified 2026-08-10 on
PostgreSQL 18.4 against a three-partition LIST-partitioned edge table. A
traversal restricted to declared and structural produces an `Append` node
containing only `edges_declared` and `edges_structural`. The asserted partition
is **absent from the plan**, so it is pruned at plan time rather than filtered at
runtime. The deterministic subgraph is not visited. G3 holds on the shipped
engine version, not only on the version the charter tested.

**Traversal must dedup on node, not enumerate paths.** Measured on the same
probe, a 6,000-edge graph with a branching factor of 2, depth capped at 20:

| Formulation | Rows | Storage | Time |
|---|---|---|---|
| `UNION ALL` with a per-path visited array | 2,097,151 | 430 MB spilled to disk | 3,563 ms |
| `UNION`, global dedup on node | 231 | 34 kB in memory | 0.44 ms |

The per-path visited set is the reference's shape and it does not bound anything.
It suppresses revisits within a single path while enumerating every distinct
path, which is exponential in depth at any branching factor above 1. `UNION`
dedups against everything already produced, which makes the walk linear in nodes
reached.

This is a contract question, not only a performance one. `UNION` returns the set
of reachable nodes and cannot return the path that reached them. Verbs that must
return a path, such as `DbGraphShortestPath`, need the enumerating form with a
hard row cap, and their specs must say so. Verbs answering reachability, such as
`DbGraphNeighbors` and the traversal default, take `UNION`.

**Polymorphic endpoints resolve for free.** The reference's `imports_edges` can
point `_to` at either a symbol or a file depending on whether resolution
succeeded. With one `nodes` table, both are a `bigint`, and the `resolved` flag
stays in the payload.

**Traversal** is a recursive CTE with an explicit visited-set and a row cap. Real
depth on the reference's live graphs is 1 to 3 and the verb caps at 20.

### Phase 4: Basis Derivation

**Basis is derived at ingest, from the analyzer, and written explicitly.** It is
not a function of edge type, and the charter proves this in the data:
`codebase_calls_edges` holds 6,129 edges attributed to rust-analyzer or libclang
and 118 with no analyzer and no resolution recorded, in the same collection. A
second graph carries no provenance attribution on any code edge at all.

The reference records `analyzer` and a three-value `analysis_tier` of semantic,
structural, or text. Basis is a different axis and the mapping between them is a
design item this PRD opens rather than closes:

| Edge origin | Proposed basis | Reasoning |
|---|---|---|
| `defines`, from AST extraction | `declared` | The file literally declares the symbol. Reading the source is the whole evidence. |
| `imports` with `resolved: true` | `declared` | The import statement is in the text. |
| `calls`, `implements`, from a language server | `structural` | An analyzer resolved it. Correct, and not readable off the page. |
| Anything from tree-sitter fallback | `structural` | Same category, lower fidelity. Fidelity is `analysis_tier`, not basis. |
| Edges with no analyzer recorded, the 118 | **Unresolved** | Needs a ruling. See Open Questions. |
| GraphSAGE or embedding-similarity output | `asserted` | Inferred. |

The 118 are residue from ingesting before a better extractor covered that path,
which is why basis is mutable by design and why the change is logged rather than
lost.

### Phase 5: Retrieval

**Keyword.** tsvector with GIN, ranked with ts_rank_cd. The bar is "better than
term coverage in Rust," which is what the reference does, not "as good as
Elasticsearch."

**Vector.** pgvector over halfvec at 2048 dimensions with an HNSW index. fp16 is
already the reference's configured default, so no precision is given up. This
replaces a Rust brute-force path that refused past 100,000 embeddings.

**Hybrid is one statement.** The reference's search verb is four round trips plus
a Rust loop. Postgres fuses what ArangoDB split, and this is charter section 12's
first falsified premise: there were no fusion-splitter verbs because nothing was
fused.

### Phase 6: Diff Log and History

Append-only, per node. Records what a node was on entering the store, every
change since, and by extension what it is now.

```sql
CREATE TABLE node_log (
    id        bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id   bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    seq       integer NOT NULL,
    diff      jsonb  NOT NULL,
    at        timestamptz NOT NULL DEFAULT now(),
    UNIQUE (node_id, seq)
);
```

The head row in `nodes` is materialized convenience so a read does not replay a
chain. If head and log disagree, the log wins, because the head is derivable
from the log and the log is not derivable from the head.

**Out of scope for this phase, named so the boundary is visible:** the two replay
axes of charter section 8.3, differential provenance and model-version replay.
Both are reads over saved states keyed by the pair of content version and model
version. The log designed here is what they will read. Building them is not this
PRD.

**Checkpoint versioning is not in this store.** A model checkpoint has no
meaningful semantic diff, so it drops to the filesystem as copy-on-write
snapshots. The application names which model version it wants and does not
orchestrate snapshots.

### Phase 7: The Sink

The orchestrator's coupling is four points: the `ArangoPool` import, an error
variant, the `db` struct field, and the constructor parameter. A sink
abstraction goes in at those four points and the 12.3k LOC upstream does not
change.

**Idempotent writes.** The reference uses `overwrite: true` with ArangoDB REPLACE
semantics, made safe by deterministic keys. In Postgres this is
`INSERT ... ON CONFLICT (graph_id, natural_key) DO UPDATE`. The determinism that
made it safe there makes it safe here.

**Enrichment protocol survives.** Symbols are written twice in one ingest run,
first by syn or the Python AST, then overwritten by a language server pass with
semantic signatures. Both passes derive the same key from the same qualified
name. ON CONFLICT DO UPDATE preserves this exactly, and the structural artifact
remains if the language server is absent or fails.

**Validation invariants become constraints where they can be.** Of the
seventeen invariants in ontology section 10, the referential ones (1, 2, 3, 8
through 11) become foreign keys, and the uniqueness ones (12, 13, 14) become
UNIQUE constraints. The count invariants (4, 5, 6) and the post-ingest-only
invariant (7) stay checks, since invariant 7 is explicitly not a real-time
constraint. The primitive invariants (15, 16, 17) become an enum plus a CHECK.
**This is a reduction in what `codebase validate` has to do at runtime, and the
spec for that verb should quantify how much.**

### The Verb Inventory

32 commands survive from the reference's 55 `DaemonCommand` variants:

| Group | Verbs |
|---|---|
| Orientation | `Orient`, `Status`, `DbHealth`, `DbCheck`, `DbStats`, `CodebaseStats` |
| Document read | `DbQuery`, `DbGet`, `DbList`, `DbCount`, `DbRecent`, `DbCollections` |
| Document write | `DbInsert`, `DbUpdate`, `DbDelete`, `DbPurge` |
| Structure | `DbCreateCollection`, `DbCreateIndex` |
| Graph | `DbGraphTraverse`, `DbGraphShortestPath`, `DbGraphNeighbors`, `DbGraphList`, `DbGraphCreate`, `DbGraphDrop`, `DbGraphMaterialize` |
| Schema | `DbSchemaInit`, `DbSchemaList`, `DbSchemaShow`, `DbSchemaVersion` |
| Embedding | `EmbedText`, `GraphEmbedEmbed`, `GraphEmbedNeighbors` |

Dropped: 18 `Task*` (methodology), 4 `Smell*` and `LinkCodeSmell` (methodology),
`DbAql` (forbidden surface).

Naming is inherited from the reference for continuity of contract, not endorsed
as final. `DbCollections` and `DbCreateCollection` carry ArangoDB vocabulary into
a relational store and should be renamed before the verb layer is built.

### The Ingestion Operation Set

The inventory above is what exists. This is what does not, and it is why the
charter says T3 is false today. Six CLI commands reach the store directly and
never construct a verb:

| Command | Reference file | Coupled lines | Destructive |
|---|---|---|---|
| `codebase ingest` | `codebase_ingest.rs` | 67 | Yes, overwrites |
| `codebase retire` | `codebase_retire.rs` | 35 | Yes |
| `codebase prune` | `codebase_prune.rs` | 36 | Yes |
| `codebase drift` | `codebase_drift.rs` | 14 | No |
| `codebase validate` | `codebase_validate.rs` | 29 | No |
| `graph-embed update` | `graph_embed_update.rs` | 18 | Yes |

Each needs a verb with a contract fixed here and an implementation built later.
A human running `codebase retire` today produces no verb-layer record of what was
swept, and that is the gap the regulated sale rests on closing.

---

## Technical Architecture

### Resolved Design Decisions

**D1. Surrogate `bigint` primary keys, natural keys kept UNIQUE.** Edges carry
two endpoints and recursive CTEs walk them repeatedly, so fixed-width integer
joins beat text joins on the hot path. The reference's deterministic keys are
what make re-ingest idempotent and they are preserved as a UNIQUE constraint,
not discarded.

*Cost measured 2026-08-10, and it is not free.* `CrateEdge.from` and `.to` are
`String`, holding collection-qualified document IDs such as
`codebase_files/src_lib_rs`. The sink therefore has to resolve text to `bigint`,
and it has to handle an edge naming a node that has not been inserted yet, since
edges and nodes are produced by different passes. The Phase 7 spec owns that
resolution strategy. The alternative, keeping text keys as the foreign key,
removes the resolution step and pays for it on every traversal hop. The
collection prefix inside those strings also stops meaning anything once `nodes`
is one table, so the sink strips it.

**D2. One `nodes` table, not two.** The reference's five primitives are already a
closed taxonomy over `codebase_files` plus `codebase_symbols`. A `kind` enum and
a JSONB payload express it in one table, which is also what lets polymorphic
edge endpoints be a plain `bigint`.

**D3. Relation is a column, basis is the partition key.** The reference makes the
collection name the relation. The charter requires LIST partitioning on basis.
Basis wins the partition key because it is the trust boundary and pruning it is
what makes the deterministic subgraph unvisited rather than filtered.

**D4. Cascade by foreign key, not by command.** Ontology section 7.3's six-step
manual cascade becomes ON DELETE CASCADE. Inherit, do not author.

**D5. tsvector as a generated stored column.** Keeps the index in sync with the
text without application code, and answers the reference's open Q1.

**D6. halfvec at 2048 dimensions.** fp16 is already the reference's default, so
this costs no precision. Measured mandatory rather than preferred: current
pgvector refuses an HNSW index on `vector(2048)` regardless of version, and only
`halfvec` clears the 2000-dimension ceiling. See Phase 1.

**D7. Traversal dedups on node by default.** `UNION`, not `UNION ALL` with a
path array. Measured at four orders of magnitude on a synthetic branching graph.
Path-returning verbs are the named exception and carry a hard row cap. See
Phase 3.

### Open Questions

**Q1. Basis for the 118 unattributed edges. Root cause found 2026-08-10, and it
was a type, not a data accident.**

Reading what the analysis layer emits, rather than the ontology document that
describes it:

| Emitted type | Carries analyzer and tier? |
|---|---|
| `FileAnalysis` | Yes, both, typed and required |
| `SymbolDocument` | Yes, both, typed and required |
| `CrateEdge` | **No.** Only `from`, `to`, `kind`, and `metadata: serde_json::Value` with `#[serde(flatten)]` |

Edge provenance in the reference can only travel inside a free-form JSON blob,
so recording it is a convention each call site has to remember. 6,129 call sites
remembered and 118 did not. Nothing in the type asked them to.

Two consequences.

The old rows need no ruling. Yeomna migrates nothing and the data is unreadable
without resurrecting ArangoDB, so refusing unattributed ingest going forward
costs nothing and the question of what to backfill never arises.

The design fix is in the emitted type, not the table. Charter 8.4's requirement
that basis be "derived at ingest and written explicitly" reads as a constraint on
storage. It is really a constraint on the producer. **The Yeomna edge type
carries `basis` and `analyzer` as required typed fields**, which makes an
unattributed edge unrepresentable rather than discouraged. A NOT NULL column
alone would only move the failure from silent to runtime.

**Q2. Graph isolation mechanism. Ruled 2026-08-12: the `graph_id` column,
one logical table set, as the DDL assumes.** The column keeps the verb layer
bind-parameter pure (schema names cannot be bind parameters, and dynamic SQL
is the defect class the reference's bind-shape lesson exists to prevent),
keeps RLS the single inherited access mechanism, keeps one diff log and one
migration story, and leaves cross-graph bridges representable with ordinary
foreign keys. What the schema-per-graph option offered (instant drop,
per-corpus dump, per-graph HNSW) is reachable later by LIST-partitioning on
`graph_id` within the same logical model if measurement demands it, a
graduation not a fork. A `graphs` registry table (id, name, created_at)
becomes real, since `DbGraphCreate`, `DbGraphList`, and `DbGraphDrop` operate
on it. Bridge-edge representation stays undesigned until the first bridge
exists, per the no-vocabulary rule.

**Q3. Verb naming.** `DbCollections`, `DbCreateCollection`, and `DbGraphMaterialize`
carry ArangoDB vocabulary into a relational store. The second option was to keep
the names for contract continuity during differential testing and rename after.
**That rationale is gone**, since there is no differential testing, so the
question reduces to picking new names. Still needs a ruling, but only one answer
is left standing.

**Q4. M3, ingest isolation.** This one is settled inside this PRD, not deferred.
See Risk Assessment.

---

## Testing Strategy

**Correction, 2026-08-10. There is no parity percentage and no oracle is
wanted.** This section was drafted with an above-95-percent behavioral target
measured against a running reference. Both halves are struck.

The reference is not running, which is the smaller reason. The larger one is
that matching its behavior was never the goal. ArangoDB was displaced over a
license, not over performance or correctness, and charter section 10 already
retired the performance comparison because the remembered baseline was never a
graph-engine result. A differential harness would spend real effort proving a
resemblance nobody is buying.

**Parity here is feature parity as a design intent.** The verb inventory is the
feature list. A verb passes when it does the job its contract names, on this
schema, correctly. It does not have to emit the bytes the reference emitted, and
where Yeomna can do the job better it should, since two of the five capabilities
are upgrades over the reference rather than ports of it.

What follows is testing of Yeomna against its own contracts. Nothing below
compares the two systems.

**Invariant tests** for the seventeen ontology invariants, including the ones now
enforced by constraints, since a constraint that silently fails to exist is worse
than a check that runs.

**Partition pruning tests.** EXPLAIN must show the asserted partition untouched
in both the base term and the recursive term of a traversal restricted to
declared and structural. The charter treats this as verified and the test exists
to keep it verified.

**Idempotency tests.** Re-ingesting the same corpus twice produces identical
row counts and identical natural keys.

**Cycle and depth tests.** Feeds M2. See Risk Assessment for what these can and
cannot conclude.

---

## Risk Assessment

### M1. Native FTS relevance. Open.

Is tsvector with ts_rank_cd good enough on a real firm's document set? Bar is
better than term coverage in Rust. Graduation is pg_search, from an active
upstream, inside the same engine. Not a second store.

Dogfooding this repository informs M1 and cannot settle it, since a code
repository is not a firm's document set.

### M2. Recursive CTE behavior at depth. Open.

Partition pruning through the recursive term is confirmed, now on 18.4. Untested
is cycle behavior and row growth at depth on a real code graph, where calls edges
do cycle. The benchmark should try to blow up the depth-20 ceiling rather than
confirm that depth 3 works.

**Early signal, from a synthetic probe and not a substitute for the benchmark.**
The depth-20 ceiling blew up on the first attempt. A 6,000-edge graph with
branching factor 2 produced 2.1 million rows and spilled 430 MB. The cause was
the query formulation rather than the engine, and D7 addresses it. Two things
follow. Row growth at depth is real and the row cap is load-bearing rather than
defensive. And the benchmark must test both traversal formulations, since the
reference's shape is the one that explodes.

Dogfooding produces the real graph. It supplies the corpus, not the answer. M2
stays open until the benchmark is run and reported.

### M3. Ingest isolation. Decided here.

`codebase retire` becomes a multi-statement transaction rather than a single
atomic statement, so its isolation level stops being something the engine picks
and becomes a decision someone makes. Write-heavy ingest runs against read-heavy
serving in one instance. The charter calls this a correctness surface where a
defect is silent.

This PRD is the place that decision gets made, and the spec for Phase 7 must name
the isolation level for every multi-statement ingestion operation and state what
anomaly the choice admits. Note that copying the reference would not have helped
here even if it were running, since the reference has the same unresolved
problem. This is one of the places where feature parity gives no guidance and
the design has to be made rather than inherited.

### M4. Erasure against an append-only history. Unsolved.

A regulated erasure obligation wants data gone. The diff log wants nothing lost.
Erasure must also reach the asserted partition, or a deleted document leaves
inferred edges pointing at a ghost. The named exception is that erasure
tombstones without destroying the chain's integrity.

**This is unsolved and this PRD does not solve it.** It is recorded here because
the schema above is what an eventual erasure path has to cut through, and
because it must not be discovered during a compliance review.

### R1. Scope creep into the verb layer.

The inventory is in scope, the implementation is not. The charter calls the verb
layer the largest single block of work in the build, and folding it into a store
PRD would hide that.

### R2. Porting the bypass.

A high-fidelity port reproduces the 25-of-57 verb-layer bypass faithfully. The
ingestion operation set above exists to make that visible as work rather than
invisible as fidelity.

---

## Timeline

Phases are ordered by dependency, not by estimate. No dates until the specs are
written, since the charter's own practice is to hold a measurement open rather
than guess at it.

| Phase | Depends on | Blocking for |
|---|---|---|
| 1. Instance and sealing | Nothing | Everything |
| 2. Nodes, chunks, embeddings | 1 | 3, 5, 7 |
| 3. Edges, basis, traversal | 2, and Q1 ruled | 4, M2 benchmark |
| 4. Basis derivation | 3 | 7 |
| 5. Retrieval | 2 | M1 benchmark |
| 6. Diff log | 2 | Replay axes, later PRD |
| 7. Sink | 2, 4, and M3 decided | Dogfooding |

Dogfooding follows Phase 7 and is where methodology reenters as corpus.

---

## Appendices

### Appendix A. Source Material

| Topic | Reference file |
|---|---|
| Ontology, five primitives, invariants | `docs/codebase-graph-ontology.md` (731 lines) |
| Declarative schema and RACE | `docs/declarative-schema.md` (644 lines) |
| Daemon protocol | `docs/daemon-protocol.md` (711 lines) |
| Embedder contract, PE-API v1 | `docs/persephone-embedding-api.md` (241 lines) |
| Operation vocabulary | `docs/model-operation-vocabulary.md` (246 lines) |
| Verb surface | `crates/hades-core/src/dispatch.rs` |
| Key derivation | `crates/hades-core/src/db/keys.rs` |
| Sink seam | `crates/hades-core/src/pipeline/orchestrator.rs` |

### Appendix B. Charter Items This PRD Touches

Folded pending a strike-or-keep ruling, per charter Appendix A: audit-log
integrity as a shipped default (built here as specified, in Phase 1).

Open and not closed here: M1, M2, M4, the final product name, whether the
reranker and categorizer are first-class stages or contract-defined empty slots,
and whether the substrate-choice control of section 12 item 5 is wanted.

Decided here: M3.
