-- Yeomna store schema, per docs/specs/008-store-schema/spec.md.
-- Idempotent: every statement tolerates re-application.
--
-- There is no in-place migration here and there will not be. A column
-- added to a CREATE TABLE below does not reach a database that already
-- exists, and the answer to that is to drop the database and re-ingest,
-- because the graph is a rebuildable index derived from source (T4). An
-- ALTER that upgrades an existing database in place is migration tooling,
-- which the charter declines to own.
-- Rulings cited inline: Q1 (typed provenance), Q2/R1 (graph_id column),
-- D1..D7 (store PRD resolved decisions), charter section 6 (audit defaults).

-- pgvector supplies halfvec and hnsw below. Installing the package does
-- not enable it per database, so a fresh database fails at the first
-- halfvec reference without this. When the extension already exists the
-- statement is a notice-level no-op with no privilege check.
CREATE EXTENSION IF NOT EXISTS vector;

-- Graphs registry (Q2). Dropping a graph cascades through everything:
-- T4 made mechanical, the graph is a rebuildable index.
CREATE TABLE IF NOT EXISTS graphs (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Nodes (D2: one table, five primitives plus document). natural_key is the
-- deterministic key from yeomna-keys (D1: bigint joins, natural key UNIQUE
-- for idempotent upsert). Everything else the emitters produce lands in
-- payload. CHECK rather than enum: ontology invariants 15/16 enforced,
-- and a new kind is a constraint edit, not a type migration.
CREATE TABLE IF NOT EXISTS nodes (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    graph_id    bigint NOT NULL REFERENCES graphs(id) ON DELETE CASCADE,
    natural_key text   NOT NULL,
    kind        text   NOT NULL CHECK (kind IN
                  ('file', 'module', 'type', 'callable', 'value', 'document')),
    payload     jsonb  NOT NULL DEFAULT '{}',
    -- R9 (spec 011): when this node's content last landed, not when it was
    -- last seen. An unchanged re-ingest does not move it, which is what
    -- makes `recent` mean recently changed.
    ingested_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (graph_id, natural_key)
);
CREATE INDEX IF NOT EXISTS nodes_payload_gin ON nodes USING gin (payload);
CREATE INDEX IF NOT EXISTS nodes_graph_kind ON nodes (graph_id, kind);

-- Chunks. start_char/end_char are byte offsets, matching TextChunk and the
-- CRLF-exact contract. tsv is the FTS column (store PRD Phase 5 reads it).
CREATE TABLE IF NOT EXISTS chunks (
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
CREATE INDEX IF NOT EXISTS chunks_tsv_gin ON chunks USING gin (tsv);
CREATE INDEX IF NOT EXISTS chunks_symbols_gin ON chunks USING gin (symbol_ids);

-- Embeddings. halfvec(2048) is mandatory, not preferred: measured on this
-- cluster, vector(2048) refuses HNSW. model_hash pairs with
-- yeomna_keys::model_hash for staleness detection.
CREATE TABLE IF NOT EXISTS embeddings (
    chunk_id   bigint PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    vec        halfvec(2048) NOT NULL,
    model      text NOT NULL,
    model_hash text NOT NULL
);
CREATE INDEX IF NOT EXISTS embeddings_hnsw ON embeddings
    USING hnsw (vec halfvec_cosine_ops);

-- Edge basis (Q1, charter 8.1). The enum arrives now that its consumer
-- exists, closing the G4 deferral as ruled.
DO $$ BEGIN
    CREATE TYPE edge_basis AS ENUM ('declared', 'structural', 'asserted');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- Edges, LIST-partitioned by basis (D3: basis is the trust boundary and
-- the partition key, status is a plain mutable column). basis and analyzer
-- are NOT NULL: Q1's ruling in DDL, an unattributed edge is
-- unrepresentable (the reference carried 118 of them because provenance
-- could be omitted).
CREATE TABLE IF NOT EXISTS edges (
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

CREATE TABLE IF NOT EXISTS edges_declared
    PARTITION OF edges FOR VALUES IN ('declared');
CREATE TABLE IF NOT EXISTS edges_structural
    PARTITION OF edges FOR VALUES IN ('structural');
CREATE TABLE IF NOT EXISTS edges_asserted
    PARTITION OF edges FOR VALUES IN ('asserted');

CREATE INDEX IF NOT EXISTS edges_src ON edges (graph_id, src_id);
CREATE INDEX IF NOT EXISTS edges_dst ON edges (graph_id, dst_id);

-- Edge identity (ruled in spec 009): an edge is its endpoints, relation,
-- and basis. analyzer, status, and payload are attributes. Every analyzer
-- already dedups on this triple at emission, so the index encodes a
-- promise the emitters make, and it is the ON CONFLICT target for edge
-- writes when H3 arrives. basis is in the key, as partitioning requires.
CREATE UNIQUE INDEX IF NOT EXISTS edges_identity
    ON edges (graph_id, src_id, dst_id, relation, basis);

-- The diff log (charter 8.2). Append-only by grant. The head row in nodes
-- is materialized convenience: if head and log disagree, the log wins.
CREATE TABLE IF NOT EXISTS node_log (
    id      bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id bigint NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    seq     integer NOT NULL,
    diff    jsonb NOT NULL,
    at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (node_id, seq)
);

-- The audit table (charter section 6, integrity as a shipped default).
-- The verb layer writes it and defines actor semantics (peercred).
-- outcome is V-Q1's attempt logging (spec 010): NULL is an attempt whose
-- completion was never recorded, which is the crash story told by the
-- schema. On completion the writer sets exactly one of 'ok' or
-- 'failed: <kind>', where <kind> is yeomna_verbs::VerbError::kind(), one
-- of not-found, invalid-args, unimplemented, denied, internal. The kind
-- alone, never the error detail: the detail rides the response envelope,
-- which carries 'kind: detail', and an audit column is a stable
-- vocabulary rather than a message log.
CREATE TABLE IF NOT EXISTS audit_log (
    id      bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at      timestamptz NOT NULL DEFAULT now(),
    actor   text NOT NULL,
    verb    text NOT NULL,
    args    jsonb NOT NULL DEFAULT '{}',
    outcome text
);

-- Grants. yeomna_app reads and writes data, appends to the logs, and can
-- never rewrite history. yeomna_audit owns the audit table.
-- Prerequisites, provisioned at cluster creation and not by this file:
-- the roles yeomna_app and yeomna_audit exist, and the applying role
-- (yeomna_owner) is a member of yeomna_audit, which ALTER TABLE ...
-- OWNER TO requires.
GRANT SELECT, INSERT, UPDATE, DELETE
    ON graphs, nodes, chunks, embeddings, edges,
       edges_declared, edges_structural, edges_asserted
    TO yeomna_app;
GRANT SELECT, INSERT ON node_log, audit_log TO yeomna_app;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO yeomna_app;
ALTER TABLE audit_log OWNER TO yeomna_audit;
GRANT INSERT ON audit_log TO yeomna_app;
REVOKE UPDATE, DELETE ON audit_log FROM yeomna_app;
-- The one exception, column-scoped (spec 010): the completion mark is
-- updatable, history is not.
GRANT UPDATE (outcome) ON audit_log TO yeomna_app;
REVOKE UPDATE, DELETE ON node_log FROM yeomna_app;
