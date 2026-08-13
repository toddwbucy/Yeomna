# Specification: 008 The Store Schema

Parent PRD: `docs/PRD-postgres-store.md`, phases 2, 3, 4, and 6, under the
rulings R1 (Q2: `graph_id` column, 2026-08-12) and R2 (`full_page_writes`
off, 2026-08-12). Fills the schema half of holes-ledger H1.
Status: draft, 2026-08-13.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. SQL keeps its syntax.

---

## Overview

The first construction of the build era: the Postgres schema, written
against the emitted types on main and deployed to the sealed cluster. A new
crate, `yeomna-store`, carries `schema.sql`, an idempotent applier, and the
integration tests that prove the load-bearing claims as automated checks
rather than one-time probes.

This spec is the schema only. The `IngestSink` implementation is spec 009,
against tables this spec creates.

The design inputs, all already in hand: the emitted types (`FileAnalysis`,
`SymbolDocument`, `CrateEdge`, `TextChunk`), the byte-stable `chunk_doc` and
`embedding_doc` JSON shapes pinned by `yeomna-pipeline` tests, the golden
keys, the store PRD's resolved decisions D1 through D7, the Q1 typed-
provenance ruling, and the Q2 `graph_id` ruling.

## The schema

Owner role runs DDL. All tables live in the default schema of database
`yeomna`.

### Graphs registry (ruled real by Q2)

```sql
CREATE TABLE graphs (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);
```

`DbGraphCreate`, `DbGraphList`, and `DbGraphDrop` operate on this table.
Dropping a graph cascades through everything below, which is T4 made
mechanical: the graph is a rebuildable index and its removal is one delete.

### Nodes (D2: one table, five primitives)

```sql
CREATE TABLE nodes (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    graph_id    bigint NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    natural_key text   NOT NULL,
    kind        text   NOT NULL CHECK (kind IN
                  ('file', 'module', 'type', 'callable', 'value', 'document')),
    payload     jsonb  NOT NULL DEFAULT '{}',
    UNIQUE (graph_id, natural_key)
);

CREATE INDEX nodes_payload_gin ON nodes USING gin (payload);
CREATE INDEX nodes_graph_kind ON nodes (graph_id, kind);
```

- `natural_key` is the deterministic key from `yeomna-keys` (D1: surrogate
  `bigint` for joins, natural key UNIQUE for idempotent upsert).
- `kind` covers the five code primitives plus `document`, since the document
  pipeline's metadata rows are nodes too. A CHECK rather than an enum:
  ontology invariants 15 and 16 enforced, and adding a kind is one
  constraint edit rather than a type migration.
- Everything the emitters produce beyond identity lands in `payload`
  (`SymbolDocument`'s fields, `FileAnalysis` metrics, the document metadata
  shape from `chunk_doc`'s sibling). The sink strips the transitional
  container prefixes from endpoint strings when resolving, per D1.

### Chunks and embeddings (PRD Phase 2, verified shapes)

```sql
CREATE TABLE chunks (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id     bigint  NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    chunk_index integer NOT NULL,
    text        text    NOT NULL,
    start_char  integer NOT NULL,
    end_char    integer NOT NULL,
    symbol_ids  bigint[] NOT NULL DEFAULT '{}',
    tsv tsvector GENERATED ALWAYS AS (to_tsvector('english', text)) STORED,
    UNIQUE (node_id, chunk_index)
);

CREATE INDEX chunks_tsv_gin ON chunks USING gin (tsv);
CREATE INDEX chunks_symbols_gin ON chunks USING gin (symbol_ids);

CREATE TABLE embeddings (
    chunk_id   bigint PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    vec        halfvec(2048) NOT NULL,
    model      text NOT NULL,
    model_hash text NOT NULL
);

CREATE INDEX embeddings_hnsw ON embeddings
    USING hnsw (vec halfvec_cosine_ops);
```

- `start_char` and `end_char` are byte offsets, matching `TextChunk` and the
  CRLF-exact contract the chunking fixes established.
- `halfvec(2048)` is mandatory, not preferred (measured on this cluster:
  `vector(2048)` refuses HNSW). `model_hash` pairs with `yeomna_keys::
  model_hash` for staleness detection.
- The FK cascade chain (graphs to nodes to chunks to embeddings) IS the
  reference ontology's six-step manual deletion cascade, inherited rather
  than authored (D4).

### Edges (PRD Phase 3, Q1 typed provenance, D3)

```sql
CREATE TYPE edge_basis AS ENUM ('declared', 'structural', 'asserted');

CREATE TABLE edges (
    id       bigint GENERATED ALWAYS AS IDENTITY,
    graph_id bigint NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    src_id   bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    dst_id   bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    relation text   NOT NULL CHECK (relation IN
               ('defines', 'calls', 'implements', 'imports', 'contains')),
    basis    edge_basis NOT NULL,
    status   text NOT NULL DEFAULT 'ratified'
               CHECK (status IN ('ratified', 'pending')),
    analyzer text NOT NULL,
    payload  jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (id, basis)
) PARTITION BY LIST (basis);

CREATE TABLE edges_declared   PARTITION OF edges FOR VALUES IN ('declared');
CREATE TABLE edges_structural PARTITION OF edges FOR VALUES IN ('structural');
CREATE TABLE edges_asserted   PARTITION OF edges FOR VALUES IN ('asserted');

CREATE INDEX edges_src ON edges (graph_id, src_id);
CREATE INDEX edges_dst ON edges (graph_id, dst_id);
```

- The `edge_basis` enum arrives now, when its consumer exists, closing the
  G4 deferral exactly as ruled.
- **`basis` and `analyzer` are NOT NULL.** This is Q1's ruling in DDL: the
  reference's 118 unattributed edges were possible because provenance could
  be omitted, and here an unattributed edge is unrepresentable. The Rust
  edge type the sink accepts (spec 009) carries both as required fields, so
  the failure is compile-time before it is constraint-time.
- `relation` gains `contains` beyond the four code relations, for the
  document-to-chunk-free structure a document graph may need. A CHECK, so
  extending costs one edit.
- `status` is a plain column (D3): ratified and pending flip in place,
  basis changes are row movement plus a log entry.
- Traversal defaults to `UNION` node-dedup (D7). Path-returning verbs use
  the enumerating form with a hard row cap. Verbs are H2, but the spec
  records the contract the indexes serve.

### The diff log (PRD Phase 6, G4-of-history)

```sql
CREATE TABLE node_log (
    id      bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    seq     integer NOT NULL,
    diff    jsonb NOT NULL,
    at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (node_id, seq)
);
```

Append-only by grant (below), the head row in `nodes` is materialized
convenience, and if head and log disagree the log wins. The replay axes of
charter 8.3 read this table later and add nothing to it now.

### The audit table (charter section 6, shipped default)

```sql
CREATE TABLE audit_log (
    id    bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at    timestamptz NOT NULL DEFAULT now(),
    actor text NOT NULL,
    verb  text NOT NULL,
    args  jsonb NOT NULL DEFAULT '{}'
);
```

Owned by `yeomna_audit`. The verb layer (H2) writes it and defines `actor`
semantics (peercred identity). The table exists now because the charter
commits the integrity mechanisms as shipped defaults, and grants are
schema-time work.

### Grants

```sql
GRANT SELECT, INSERT, UPDATE, DELETE
    ON graphs, nodes, chunks, embeddings, edges,
       edges_declared, edges_structural, edges_asserted, node_log
    TO yeomna_app;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO yeomna_app;

ALTER TABLE audit_log OWNER TO yeomna_audit;
GRANT INSERT ON audit_log TO yeomna_app;
REVOKE UPDATE, DELETE ON audit_log FROM PUBLIC, yeomna_app;

-- node_log is append-only for the app role.
REVOKE UPDATE, DELETE ON node_log FROM yeomna_app;
```

## The crate: `yeomna-store`

- `schema.sql` embedded via `include_str!`, applied by an idempotent
  `apply_schema` function (CREATE IF NOT EXISTS discipline, or a
  drop-and-create dev mode gated behind an explicit flag).
- Driver: `tokio-postgres` over the Unix socket. No connection pooling yet,
  no ORM ever.
- Integration tests gated on `YEOMNA_TEST_DB` (the socket dir), **skip when
  absent** per the probe pattern, so G2 holds for every other crate and the
  workspace tests pass on a box with no cluster.

## The claims that become automated tests

1. **Partition pruning survives the recursive term.** The EXPLAIN probe from
   2026-08-10 becomes a test: a traversal restricted to declared and
   structural must not name `edges_asserted` in its plan.
2. **`halfvec(2048)` HNSW-indexes and `vector(2048)` refuses.** The measured
   correction to the charter, pinned.
3. **Unattributed edges are unrepresentable.** Inserting an edge with null
   basis or analyzer fails.
4. **The cascade is complete.** Deleting a graph leaves zero rows in every
   table. Deleting a node clears its chunks, embeddings, log entries, and
   edges.
5. **Idempotent upsert works as the sink will use it.** `INSERT ... ON
   CONFLICT (graph_id, natural_key) DO UPDATE` twice with the same golden
   key yields one row.
6. **Append-only holds.** `yeomna_app` can INSERT into `node_log` and
   `audit_log` and cannot UPDATE or DELETE either.
7. **The pinned JSON shapes land.** A `chunk_doc` and `embedding_doc` pair
   from the pipeline's own test constructors round-trips into `chunks` and
   `embeddings` losslessly.

## Out of Scope

- The `IngestSink` implementation (spec 009), verbs, RLS policies (verb-
  layer era, `SET ROLE` design), the replay-axes cache, bridge-edge
  representation (no bridge exists), M2's cycle benchmark (dogfooding
  supplies the corpus), and any Python.

## Success Criteria

1. Workspace gate green with no cluster present (tests skip, stated).
2. Against the dev cluster: schema applies idempotently, all seven claim
   tests pass.
3. The schema file carries the ruling citations as comments (Q1, Q2, D1
   through D7), so the DDL explains itself.
4. Store-brand greps stay clean.
5. Review notes map every emitted-type field to its column or its `payload`
   residence.
