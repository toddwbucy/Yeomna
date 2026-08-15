//! `PgSink`: the store's implementation of the pipeline's write boundary.
//!
//! Built per `docs/specs/009-ingest-sink/spec.md`. The trait
//! (`yeomna_pipeline::sink::IngestSink`) was designed from its one caller,
//! the schema (spec 008) from the emitted types, and this module is the
//! join: JSON documents addressed by the transitional container names of
//! `yeomna_pipeline::profile`, landing in tables those names stop meaning
//! anything against.
//!
//! One sink, one graph, one connection. The orchestrator awaits each call
//! before the next, and this implementation relies on that: it manages
//! transactions with explicit BEGIN/SAVEPOINT/COMMIT on a shared `&Client`,
//! so interleaving two concurrent batches on one sink is not supported.
//! Concurrent ingest is M3's question, restated in the spec, not defended
//! here.

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio_postgres::Client;
use yeomna_pipeline::sink::{IngestSink, InsertOutcome};

use crate::StoreError;

/// Where a container name routes (spec 009, extended by spec 011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// A metadata container: rows become `nodes` of this fixed kind.
    Metadata(&'static str),
    /// A symbol container: rows become `nodes` whose kind the document
    /// carries, since one container holds all four code primitives.
    Symbol,
    /// A chunk container: rows become `chunks`.
    Chunks,
    /// An embedding container: rows become `embeddings`.
    Embeddings,
    /// An edge container: rows become `edges`, endpoints resolved here
    /// (D1: the sink owns text to id resolution).
    Edges,
}

/// Map a transitional container name to its table. `None` is a caller bug.
fn route(container: &str) -> Option<Route> {
    match container {
        "documents" => Some(Route::Metadata("document")),
        "codebase_files" => Some(Route::Metadata("file")),
        "codebase_symbols" => Some(Route::Symbol),
        "chunks" | "codebase_chunks" => Some(Route::Chunks),
        "embeddings" | "codebase_embeddings" => Some(Route::Embeddings),
        "edges" | "codebase_edges" => Some(Route::Edges),
        _ => None,
    }
}

/// Strip a collection-qualified prefix from an endpoint id (D1). Only
/// recognized container names are stripped, so a natural key that happens
/// to contain a slash survives intact.
fn strip_container(id: &str) -> &str {
    match id.split_once('/') {
        Some((prefix, rest)) if route(prefix).is_some() => rest,
        _ => id,
    }
}

/// Field names that denote the parent document key in
/// `remove_documents_by_fields`. Anything else is refused: a sink that
/// guessed would delete the wrong rows silently.
const PARENT_KEY_FIELDS: [&str; 3] = ["doc_key", "parent_key", "file_key"];

/// Parse the chunk index from a pinned-format chunk key and validate by
/// reconstruction through `yeomna_keys::chunk_key`. Never trust a parsed
/// index that does not round-trip (spec 009 EC-2).
fn chunk_index_from_key(doc_key: &str, chunk_key: &str) -> Option<usize> {
    let suffix = chunk_key.rsplit("_chunk_").next()?;
    let index: usize = suffix.parse().ok()?;
    (yeomna_keys::chunk_key(doc_key, index) == chunk_key).then_some(index)
}

/// The store-backed ingest sink for one graph.
pub struct PgSink {
    client: Client,
    graph_id: i64,
}

impl PgSink {
    /// Wrap a connection and resolve (or create) the named graph.
    ///
    /// The connection should carry the runtime role (`yeomna_app`), whose
    /// grants cover everything this sink does. `apply_schema` stays owner
    /// work and is not this type's concern.
    ///
    /// The client must be idle and dedicated to this sink. Batch inserts
    /// issue explicit BEGIN and COMMIT, so a connection carrying a
    /// caller-owned open transaction would have that transaction committed
    /// or rolled back from under it.
    pub async fn new(client: Client, graph_name: &str) -> Result<Self, StoreError> {
        // DO UPDATE rather than DO NOTHING so RETURNING yields the id on
        // the already-exists path too.
        let graph_id = client
            .query_one(
                "INSERT INTO graphs (name) VALUES ($1)
                 ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id",
                &[&graph_name],
            )
            .await?
            .get(0);
        Ok(Self { client, graph_id })
    }

    /// The graph this sink writes into.
    pub fn graph_id(&self) -> i64 {
        self.graph_id
    }

    /// Resolve a parent node id by natural key, memoized per batch.
    ///
    /// Returns the driver error directly so callers inside the savepoint
    /// path can `?` it: only a missing parent is `Ok(None)`, a transport
    /// failure propagates and aborts the batch.
    async fn parent_node(
        &self,
        cache: &mut HashMap<String, Option<i64>>,
        doc_key: &str,
    ) -> Result<Option<i64>, tokio_postgres::Error> {
        if let Some(hit) = cache.get(doc_key) {
            return Ok(*hit);
        }
        let id = self
            .client
            .query_opt(
                "SELECT id FROM nodes WHERE graph_id = $1 AND natural_key = $2",
                &[&self.graph_id, &doc_key],
            )
            .await?
            .map(|r| r.get(0));
        cache.insert(doc_key.to_string(), id);
        Ok(id)
    }

    /// Insert one document inside its savepoint. `Ok(true)` created or
    /// replaced a row, `Ok(false)` is a counted per-document rejection
    /// (malformed, unknown parent, duplicate under `overwrite: false`).
    /// `Err` aborts the batch (connection-level failure).
    async fn insert_one(
        &self,
        route: Route,
        doc: &Value,
        overwrite: bool,
        cache: &mut HashMap<String, Option<i64>>,
    ) -> Result<bool, StoreError> {
        self.client.batch_execute("SAVEPOINT sink_doc").await?;
        let outcome = self.insert_one_inner(route, doc, overwrite, cache).await;
        match outcome {
            Ok(created) => {
                self.client
                    .batch_execute("RELEASE SAVEPOINT sink_doc")
                    .await?;
                Ok(created)
            }
            Err(e) => {
                // A database rejection (constraint, dimension, cast) rolls
                // back this document alone and counts as its error. Only a
                // failure to roll back is a batch failure.
                if e.as_db_error().is_some() {
                    self.client
                        .batch_execute("ROLLBACK TO SAVEPOINT sink_doc")
                        .await?;
                    Ok(false)
                } else {
                    Err(StoreError::Db(e))
                }
            }
        }
    }

    async fn insert_one_inner(
        &self,
        route: Route,
        doc: &Value,
        overwrite: bool,
        cache: &mut HashMap<String, Option<i64>>,
    ) -> Result<bool, tokio_postgres::Error> {
        match route {
            Route::Metadata(kind) => self.insert_node(kind, doc, overwrite).await,
            Route::Symbol => {
                // One container, four primitives, so the kind rides the
                // document. An unknown kind dies on the table's CHECK and
                // counts like any other rejection.
                let Some(kind) = doc.get("kind").and_then(Value::as_str) else {
                    return Ok(false);
                };
                let kind = kind.to_string();
                self.insert_node(&kind, doc, overwrite).await
            }
            Route::Chunks => self.insert_chunk(doc, overwrite, cache).await,
            Route::Embeddings => self.insert_embedding(doc, overwrite, cache).await,
            Route::Edges => self.insert_edge(doc, overwrite, cache).await,
        }
    }

    /// Node document: `_key` becomes `natural_key`, everything else lands
    /// in `payload` verbatim.
    ///
    /// R8 (spec 011): the diff log is appended only when the payload
    /// actually differs from the head, so an unchanged re-ingest leaves the
    /// history alone. Postgres does the comparison, since a jsonb round
    /// trip through serde would report differences that are only
    /// formatting.
    async fn insert_node(
        &self,
        kind: &str,
        doc: &Value,
        overwrite: bool,
    ) -> Result<bool, tokio_postgres::Error> {
        let Some(key) = doc.get("_key").and_then(Value::as_str) else {
            return Ok(false);
        };
        let mut payload = doc.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("_key");
        }
        let payload_text = payload.to_string();

        // The head as it stands, read before the write so the log entry can
        // say what changed.
        let head = self
            .client
            .query_opt(
                "SELECT id, payload::text, (payload IS DISTINCT FROM $3::text::jsonb)
                 FROM nodes WHERE graph_id = $1 AND natural_key = $2",
                &[&self.graph_id, &key, &payload_text],
            )
            .await?;

        let sql = if overwrite {
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, $2, $3, $4::text::jsonb)
             ON CONFLICT (graph_id, natural_key)
             DO UPDATE SET kind = EXCLUDED.kind,
                           payload = EXCLUDED.payload,
                           -- R9 means when the content last landed, so an
                           -- unchanged overwrite must not move it. The
                           -- orchestrator's hash-skip hides this, but the
                           -- document flow has no hash-skip and would.
                           ingested_at = CASE
                               WHEN nodes.payload IS DISTINCT FROM EXCLUDED.payload
                               THEN now() ELSE nodes.ingested_at END
             RETURNING id"
        } else {
            "INSERT INTO nodes (graph_id, natural_key, kind, payload)
             VALUES ($1, $2, $3, $4::text::jsonb)
             ON CONFLICT (graph_id, natural_key) DO NOTHING
             RETURNING id"
        };
        let Some(row) = self
            .client
            .query_opt(sql, &[&self.graph_id, &key, &kind, &payload_text])
            .await?
        else {
            // DO NOTHING against an existing key: the duplicate the
            // reference's import semantics count.
            return Ok(false);
        };
        let node_id: i64 = row.get(0);

        match head {
            None => {
                self.append_log(node_id, &json!({"op": "insert", "to": payload}))
                    .await?;
            }
            Some(h) if h.get::<_, bool>(2) => {
                let old: Value =
                    serde_json::from_str(&h.get::<_, String>(1)).unwrap_or(Value::Null);
                self.append_log(
                    node_id,
                    &json!({"op": "update", "from": old, "to": payload}),
                )
                .await?;
            }
            // Unchanged: the write still happened, so ingested_at moved,
            // and the history stays quiet. That is R8.
            Some(_) => {}
        }
        Ok(true)
    }

    /// Append one entry to a node's history. Sequence is per node and
    /// derived here, which is safe under the one-sequential-caller rule
    /// this sink already documents and which M3 owns.
    async fn append_log(&self, node_id: i64, diff: &Value) -> Result<(), tokio_postgres::Error> {
        self.client
            .execute(
                "INSERT INTO node_log (node_id, seq, diff)
                 VALUES ($1,
                         (SELECT COALESCE(MAX(seq), 0) + 1 FROM node_log WHERE node_id = $1),
                         $2::text::jsonb)",
                &[&node_id, &diff.to_string()],
            )
            .await?;
        Ok(())
    }

    /// Edge document: endpoints arrive as natural keys, optionally
    /// collection-qualified, and resolve to `bigint` here (D1). An endpoint
    /// naming a node that does not exist is a counted rejection rather than
    /// a silent drop (spec 011 FR 6).
    async fn insert_edge(
        &self,
        doc: &Value,
        overwrite: bool,
        cache: &mut HashMap<String, Option<i64>>,
    ) -> Result<bool, tokio_postgres::Error> {
        let (Some(from), Some(to), Some(relation), Some(basis), Some(analyzer)) = (
            doc.get("from").and_then(Value::as_str),
            doc.get("to").and_then(Value::as_str),
            doc.get("relation").and_then(Value::as_str),
            doc.get("basis").and_then(Value::as_str),
            doc.get("analyzer").and_then(Value::as_str),
        ) else {
            return Ok(false);
        };
        let Some(src) = self.parent_node(cache, strip_container(from)).await? else {
            return Ok(false);
        };
        let Some(dst) = self.parent_node(cache, strip_container(to)).await? else {
            return Ok(false);
        };
        let payload = doc
            .get("payload")
            .cloned()
            .unwrap_or_else(|| json!({}))
            .to_string();
        let sql = if overwrite {
            // The identity index from spec 009 is the conflict target, and
            // attributes are what a second writer updates.
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer, payload)
             VALUES ($1, $2, $3, $4, $5::text::edge_basis, $6, $7::text::jsonb)
             ON CONFLICT (graph_id, src_id, dst_id, relation, basis)
             DO UPDATE SET analyzer = EXCLUDED.analyzer, payload = EXCLUDED.payload"
        } else {
            "INSERT INTO edges (graph_id, src_id, dst_id, relation, basis, analyzer, payload)
             VALUES ($1, $2, $3, $4, $5::text::edge_basis, $6, $7::text::jsonb)
             ON CONFLICT (graph_id, src_id, dst_id, relation, basis) DO NOTHING"
        };
        let affected = self
            .client
            .execute(
                sql,
                &[
                    &self.graph_id,
                    &src,
                    &dst,
                    &relation,
                    &basis,
                    &analyzer,
                    &payload,
                ],
            )
            .await?;
        Ok(affected == 1)
    }

    /// Chunk document: columns take index, text, and byte offsets. `_key`
    /// and `total_chunks` are derivable and not stored (spec 009).
    /// `symbol_ids` stays empty until H3.
    async fn insert_chunk(
        &self,
        doc: &Value,
        overwrite: bool,
        cache: &mut HashMap<String, Option<i64>>,
    ) -> Result<bool, tokio_postgres::Error> {
        let (Some(doc_key), Some(index), Some(text), Some(start), Some(end)) = (
            doc.get("doc_key").and_then(Value::as_str),
            doc.get("chunk_index").and_then(Value::as_i64),
            doc.get("text").and_then(Value::as_str),
            doc.get("start_char").and_then(Value::as_i64),
            doc.get("end_char").and_then(Value::as_i64),
        ) else {
            return Ok(false);
        };
        let Some(node_id) = self.parent_node(cache, doc_key).await? else {
            return Ok(false);
        };
        // Checked conversions: an out-of-range value is a counted
        // rejection, never a silently wrapped write.
        let (Ok(index), Ok(start), Ok(end)) = (
            i32::try_from(index),
            i32::try_from(start),
            i32::try_from(end),
        ) else {
            return Ok(false);
        };
        // Spec 011 FR 4: the chunk names the symbols it covers by key, and
        // only the sink knows their ids. An unresolvable key is dropped
        // from the array rather than failing the chunk, since a chunk with
        // incomplete linkage is still a correct chunk.
        let mut symbol_ids: Vec<i64> = Vec::new();
        if let Some(keys) = doc.get("symbol_keys").and_then(Value::as_array) {
            for k in keys.iter().filter_map(Value::as_str) {
                if let Some(id) = self.parent_node(cache, k).await? {
                    symbol_ids.push(id);
                }
            }
        }
        let sql = if overwrite {
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char, symbol_ids)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (node_id, chunk_index)
             DO UPDATE SET text = EXCLUDED.text,
                           start_char = EXCLUDED.start_char,
                           end_char = EXCLUDED.end_char,
                           symbol_ids = EXCLUDED.symbol_ids"
        } else {
            "INSERT INTO chunks (node_id, chunk_index, text, start_char, end_char, symbol_ids)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (node_id, chunk_index) DO NOTHING"
        };
        let affected = self
            .client
            .execute(sql, &[&node_id, &index, &text, &start, &end, &symbol_ids])
            .await?;
        Ok(affected == 1)
    }

    /// Embedding document: the chunk resolves by `(node_id, chunk_index)`
    /// with the index parsed from the pinned chunk key, the model comes
    /// from the parent node's payload (written first, per the five-call
    /// order), and `model_hash` derives through `yeomna_keys`.
    async fn insert_embedding(
        &self,
        doc: &Value,
        overwrite: bool,
        cache: &mut HashMap<String, Option<i64>>,
    ) -> Result<bool, tokio_postgres::Error> {
        let (Some(doc_key), Some(chunk_key), Some(embedding)) = (
            doc.get("doc_key").and_then(Value::as_str),
            doc.get("chunk_key").and_then(Value::as_str),
            doc.get("embedding").and_then(Value::as_array),
        ) else {
            return Ok(false);
        };
        let Some(index) = chunk_index_from_key(doc_key, chunk_key) else {
            return Ok(false);
        };
        let Ok(index) = i32::try_from(index) else {
            return Ok(false);
        };
        let Some(node_id) = self.parent_node(cache, doc_key).await? else {
            return Ok(false);
        };
        let Some(chunk_row) = self
            .client
            .query_opt(
                "SELECT id FROM chunks WHERE node_id = $1 AND chunk_index = $2",
                &[&node_id, &index],
            )
            .await?
        else {
            return Ok(false);
        };
        let chunk_id: i64 = chunk_row.get(0);
        let Some(model) = self
            .client
            .query_one(
                "SELECT payload->>'embedding_model' FROM nodes WHERE id = $1",
                &[&node_id],
            )
            .await?
            .get::<_, Option<String>>(0)
        else {
            // EC-1: a parent without a recorded model is data corruption
            // worth surfacing per document.
            return Ok(false);
        };
        // A non-numeric element is an explicit rejection, not a NaN
        // smuggled into the literal for halfvec to refuse obscurely.
        let Some(values) = embedding
            .iter()
            .map(|v| v.as_f64().map(|f| f.to_string()))
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(false);
        };
        let literal = format!("[{}]", values.join(","));
        let sql = if overwrite {
            "INSERT INTO embeddings (chunk_id, vec, model, model_hash)
             VALUES ($1, $2::text::halfvec, $3, $4)
             ON CONFLICT (chunk_id)
             DO UPDATE SET vec = EXCLUDED.vec,
                           model = EXCLUDED.model,
                           model_hash = EXCLUDED.model_hash"
        } else {
            "INSERT INTO embeddings (chunk_id, vec, model, model_hash)
             VALUES ($1, $2::text::halfvec, $3, $4)
             ON CONFLICT (chunk_id) DO NOTHING"
        };
        let affected = self
            .client
            .execute(
                sql,
                &[
                    &chunk_id,
                    &literal,
                    &model,
                    &yeomna_keys::model_hash(&model),
                ],
            )
            .await?;
        Ok(affected == 1)
    }
}

impl IngestSink for PgSink {
    type Error = StoreError;

    /// One transaction per batch, one savepoint per document (spec 009's
    /// M3 statement: READ COMMITTED, batch survivors commit, a failing
    /// document rolls back alone and counts in `errors`).
    async fn insert_documents(
        &self,
        container: &str,
        documents: &[Value],
        overwrite: bool,
    ) -> Result<InsertOutcome, StoreError> {
        let route =
            route(container).ok_or_else(|| StoreError::UnknownContainer(container.to_string()))?;
        if documents.is_empty() {
            return Ok(InsertOutcome {
                created: 0,
                errors: 0,
            });
        }
        let mut cache = HashMap::new();
        let mut created = 0;
        let mut errors = 0;
        self.client.batch_execute("BEGIN").await?;
        for doc in documents {
            match self.insert_one(route, doc, overwrite, &mut cache).await {
                Ok(true) => created += 1,
                Ok(false) => errors += 1,
                Err(e) => {
                    let _ = self.client.batch_execute("ROLLBACK").await;
                    return Err(e);
                }
            }
        }
        self.client.batch_execute("COMMIT").await?;
        Ok(InsertOutcome { created, errors })
    }

    /// Every field name the caller passes denotes the parent document key
    /// in both registered profiles, so this operation is "remove this
    /// document's rows from this container." A missing parent is a no-op:
    /// removal is idempotent and a first-run overwrite deletes nothing.
    async fn remove_documents_by_fields(
        &self,
        container: &str,
        fields: &[&str],
        key: &str,
    ) -> Result<(), StoreError> {
        // An empty slice would pass the unknown-field scan and delete with
        // no declared key field, so the contract check rejects it too.
        if fields.is_empty() {
            return Err(StoreError::UnknownRemovalField(String::new()));
        }
        if let Some(bad) = fields.iter().find(|f| !PARENT_KEY_FIELDS.contains(f)) {
            return Err(StoreError::UnknownRemovalField((*bad).to_string()));
        }
        let sql = match route(container) {
            Some(Route::Chunks) => {
                "DELETE FROM chunks USING nodes
                 WHERE chunks.node_id = nodes.id
                   AND nodes.graph_id = $1 AND nodes.natural_key = $2"
            }
            Some(Route::Embeddings) => {
                "DELETE FROM embeddings USING chunks, nodes
                 WHERE embeddings.chunk_id = chunks.id
                   AND chunks.node_id = nodes.id
                   AND nodes.graph_id = $1 AND nodes.natural_key = $2"
            }
            // EC-4: the caller never removes from a metadata container
            // through this operation, and an unknown name is the same bug.
            _ => return Err(StoreError::UnknownContainer(container.to_string())),
        };
        self.client.execute(sql, &[&self.graph_id, &key]).await?;
        Ok(())
    }
}

impl yeomna_pipeline::probe::IngestProbe for PgSink {
    type Error = StoreError;

    /// The hash lives in the file node's payload, which is where the
    /// orchestrator put it. `None` covers both "never ingested" and
    /// "ingested before the field existed", and both mean the same thing
    /// to a caller deciding whether to skip: do the work.
    async fn stored_symbol_hash(&self, natural_key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .client
            .query_opt(
                "SELECT payload->>'symbol_hash' FROM nodes
                 WHERE graph_id = $1 AND natural_key = $2",
                &[&self.graph_id, &natural_key],
            )
            .await?
            .and_then(|r| r.get(0)))
    }

    async fn enrichment_present(&self) -> Result<bool, StoreError> {
        Ok(self
            .client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM nodes
                 WHERE graph_id = $1 AND payload->>'enriched' = 'true')",
                &[&self.graph_id],
            )
            .await?
            .get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_routing_is_the_closed_profile_vocabulary() {
        assert_eq!(route("documents"), Some(Route::Metadata("document")));
        assert_eq!(route("codebase_files"), Some(Route::Metadata("file")));
        assert_eq!(route("chunks"), Some(Route::Chunks));
        assert_eq!(route("codebase_chunks"), Some(Route::Chunks));
        assert_eq!(route("embeddings"), Some(Route::Embeddings));
        assert_eq!(route("codebase_embeddings"), Some(Route::Embeddings));
        assert_eq!(route("symbols"), None);
        assert_eq!(route(""), None);
    }

    #[test]
    fn chunk_index_parses_against_the_golden_format() {
        // Round-trips through the pinned yeomna-keys format.
        let key = yeomna_keys::chunk_key("docA", 3);
        assert_eq!(key, "docA_chunk_3");
        assert_eq!(chunk_index_from_key("docA", &key), Some(3));
        // A doc key that itself contains the separator still round-trips.
        let tricky = yeomna_keys::chunk_key("a_chunk_9", 2);
        assert_eq!(chunk_index_from_key("a_chunk_9", &tricky), Some(2));
        // EC-2: wrong parent, wrong index, or junk never round-trips.
        assert_eq!(chunk_index_from_key("docB", &key), None);
        assert_eq!(chunk_index_from_key("docA", "docA_chunk_x"), None);
        // Leading zeros parse as an integer but never reconstruct.
        assert_eq!(chunk_index_from_key("docA", "docA_chunk_007"), None);
        assert_eq!(chunk_index_from_key("docA", "docA_chunk_"), None);
        assert_eq!(chunk_index_from_key("docA", "unrelated"), None);
    }
}
