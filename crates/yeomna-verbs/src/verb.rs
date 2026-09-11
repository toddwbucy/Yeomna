//! The closed vocabulary: 40 verbs, wire names per the spec 010 R4 table.
//!
//! Wire form: `{"verb": "<wire name>", "args": {...}}`. Every request
//! carries `args`, including the ones that take nothing, which send an
//! empty object. Unknown fields are denied at both levels: beside `args`
//! by the enum's own attribute, and inside it by every request struct.
//! The two together are what make a client-supplied `actor` unspeakable
//! (PRD V3), and the enum-level half is not free: serde's adjacent
//! tagging accepts unknown siblings unless told otherwise, measured
//! 2026-08-14.

use serde::{Deserialize, Serialize};

/// A convenience macro would hide the table this file IS, so the variants
/// are written out. The `wire_name` method and the tests treat this enum
/// as the R4 close made executable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", content = "args", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Verb {
    // -- Orientation (Phase 2) --------------------------------------------
    /// Per-graph survey (V-Q3): what a KG is about and where it stands.
    #[serde(rename = "orient")]
    Orient(OrientRequest),
    /// Appliance liveness and identity.
    #[serde(rename = "status")]
    Status(Empty),
    /// Data integrity and health checks.
    #[serde(rename = "health")]
    Health(Empty),
    /// Does a document exist.
    #[serde(rename = "check")]
    Check(CheckRequest),
    /// Store statistics.
    #[serde(rename = "stats")]
    Stats(StatsRequest),
    /// Codebase-graph statistics.
    #[serde(rename = "codebase.stats")]
    CodebaseStats(GraphScoped),

    // -- Read (Phase 2) ---------------------------------------------------
    /// Bounded search: structured filter, never SQL (PRD non-goal 1).
    #[serde(rename = "query")]
    Query(QueryRequest),
    /// Fetch one document by kind and key.
    #[serde(rename = "get")]
    Get(KindKey),
    /// List documents.
    #[serde(rename = "list")]
    List(ListRequest),
    /// Count documents.
    #[serde(rename = "count")]
    Count(CountRequest),
    /// Recently ingested documents.
    #[serde(rename = "recent")]
    Recent(RecentRequest),

    // -- Write (Phase 4) --------------------------------------------------
    /// Insert one document.
    #[serde(rename = "insert")]
    Insert(WriteRequest),
    /// Update one document (diff-logged).
    #[serde(rename = "update")]
    Update(WriteRequest),
    /// Delete one document.
    #[serde(rename = "delete")]
    Delete(KindKey),
    /// Remove a document and all its related data.
    #[serde(rename = "purge")]
    Purge(PurgeRequest),

    // -- Edges (Phase 4, R18) ---------------------------------------------
    /// Write one asserted edge between two existing nodes. Hand-written
    /// edges are basis `asserted` by definition, so there is no basis
    /// field to send.
    #[serde(rename = "edge.assert")]
    EdgeAssert(EdgeAssertRequest),
    /// Remove one asserted edge. Declared and structural edges belong to
    /// ingest and are refused.
    #[serde(rename = "edge.retract")]
    EdgeRetract(EdgeRetractRequest),

    // -- Graph (Phase 3) --------------------------------------------------
    /// Recursive traversal, D7 node-dedup form.
    #[serde(rename = "graph.traverse")]
    GraphTraverse(TraverseRequest),
    /// One-hop neighbors.
    #[serde(rename = "graph.neighbors")]
    GraphNeighbors(NeighborsRequest),
    /// Shortest path, the enumerating exception with its hard cap.
    #[serde(rename = "graph.shortest-path")]
    GraphShortestPath(ShortestPathRequest),
    /// List graphs in the registry.
    #[serde(rename = "graph.list")]
    GraphList(Empty),
    /// Create a graph.
    #[serde(rename = "graph.create")]
    GraphCreate(GraphName),
    /// Drop a graph and everything in it (T4 made mechanical).
    #[serde(rename = "graph.drop")]
    GraphDrop(DropRequest),
    /// Materialize derived graph state.
    #[serde(rename = "graph.materialize")]
    GraphMaterialize(GraphScoped),

    // -- Schema (Phase 3 lifecycle era, H7) --------------------------------
    /// Apply the shipped schema, idempotently (absorbs the old init).
    #[serde(rename = "schema.apply")]
    SchemaApply(SchemaApplyRequest),
    /// List known schema patterns.
    #[serde(rename = "schema.list")]
    SchemaList(Empty),
    /// Show the live structure (absorbs collections and index-status).
    #[serde(rename = "schema.show")]
    SchemaShow(Empty),
    /// Schema version.
    #[serde(rename = "schema.version")]
    SchemaVersion(Empty),

    // -- Database (V-Q4, Phases 3 and 4) -----------------------------------
    /// List databases on the cluster.
    #[serde(rename = "database.list")]
    DatabaseList(Empty),
    /// Create a database: kg pattern applies the schema at birth, plain
    /// comes up empty (V-Q4: the yeomna pattern is stampable).
    #[serde(rename = "database.create")]
    DatabaseCreate(DatabaseCreateRequest),
    /// Drop a database. The customer's box, the customer's call.
    #[serde(rename = "database.drop")]
    DatabaseDrop(DropRequest),
    /// Scoped raw SQL against plain databases only. The refusal of
    /// KG-pattern targets is runtime policy (Phase 4), not a type-level
    /// rule (spec 010 EC-3).
    #[serde(rename = "sql")]
    Sql(SqlRequest),

    // -- Embedding (Phase 2 for text, H9 for graph-embed) -------------------
    /// Embed text through the appliance's embedder.
    #[serde(rename = "embed.text")]
    EmbedText(EmbedTextRequest),
    /// Structural embedding for one node (unimplemented until H9).
    #[serde(rename = "graph-embed.embed")]
    GraphEmbedEmbed(GraphKey),
    /// Structural-embedding neighbors (unimplemented until H9).
    #[serde(rename = "graph-embed.neighbors")]
    GraphEmbedNeighbors(GraphEmbedNeighborsRequest),
    /// Refresh structural embeddings (the sixth unverbed operation).
    #[serde(rename = "graph-embed.update")]
    GraphEmbedUpdate(GraphEmbedUpdateRequest),

    // -- Ingestion (Phase 6, wrapping H3) -----------------------------------
    /// Ingest a document corpus.
    #[serde(rename = "ingest")]
    Ingest(IngestRequest),
    /// Ingest a codebase (absorbs the captured `codebase update`).
    #[serde(rename = "codebase.ingest")]
    CodebaseIngest(IngestRequest),
    /// Retire ingested code by path prefix, naming what was swept.
    #[serde(rename = "codebase.retire")]
    CodebaseRetire(RetireRequest),
    /// Prune orphaned rows, naming what was swept.
    #[serde(rename = "codebase.prune")]
    CodebasePrune(DropScoped),
    /// Report drift between the graph and the working tree.
    #[serde(rename = "codebase.drift")]
    CodebaseDrift(DriftRequest),
    /// Validate graph invariants not already held by constraints.
    #[serde(rename = "codebase.validate")]
    CodebaseValidate(GraphScoped),
}

/// The empty request, for verbs that take nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Empty {}

/// A request scoped to one graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphScoped {
    pub graph: String,
}

/// A graph name for lifecycle verbs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphName {
    pub name: String,
}

/// A node addressed within a graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphKey {
    pub graph: String,
    pub key: String,
}

/// A document addressed by kind and key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KindKey {
    pub kind: String,
    pub key: String,
}

/// One asserted edge to write (R18). Both endpoints are node keys in the
/// session's graph. The relation is the caller's vocabulary, validated as
/// an identifier and deliberately not enumerated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EdgeAssertRequest {
    pub from: String,
    pub to: String,
    pub relation: String,
    /// Edge attributes, empty when absent. Asserting an existing edge
    /// again replaces these, which makes assertion a restatement rather
    /// than an error.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

/// One asserted edge to remove (R18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EdgeRetractRequest {
    pub from: String,
    pub to: String,
    pub relation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OrientRequest {
    /// Survey this graph, or all graphs when absent (V-Q3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CheckRequest {
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QueryRequest {
    pub search_text: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Restrict to one node kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Fuse vector and keyword ranking.
    #[serde(default)]
    pub hybrid: bool,
    /// Add structural graph ranking.
    #[serde(default)]
    pub structural: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default = "default_page")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
    /// Restrict to children of this document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CountRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RecentRequest {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WriteRequest {
    pub kind: String,
    pub key: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PurgeRequest {
    pub key: String,
    /// Skip confirmation. The daemon has no prompt, so this is the
    /// CLI-facing acknowledgement carried through for the audit args.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TraverseRequest {
    pub graph: String,
    pub start: String,
    /// Relation filter, empty means all.
    #[serde(default)]
    pub relations: Vec<String>,
    /// Basis filter, empty means all. Declared plus structural prunes the
    /// asserted partition (claim 1).
    #[serde(default)]
    pub bases: Vec<String>,
    #[serde(default = "default_depth")]
    pub depth: u32,
    #[serde(default = "default_row_cap")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NeighborsRequest {
    pub graph: String,
    pub key: String,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub relations: Vec<String>,
    #[serde(default)]
    pub bases: Vec<String>,
    #[serde(default = "default_page")]
    pub limit: u32,
}

/// Edge direction for neighbor queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Direction {
    Out,
    In,
    #[default]
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ShortestPathRequest {
    pub graph: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub relations: Vec<String>,
    #[serde(default)]
    pub bases: Vec<String>,
    /// The hard cap on the enumerating form (D7's named exception). It
    /// bounds both the walk, through the fetch limit that stops the
    /// recursion, and the result: a search that hits it reports
    /// truncated.
    #[serde(default = "default_row_cap")]
    pub cap: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DropRequest {
    pub name: String,
    /// The audit-carried acknowledgement for a destructive verb.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SchemaApplyRequest {
    /// Target database, defaulting to the session's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DatabaseCreateRequest {
    pub name: String,
    pub kind: DatabaseKind,
}

/// What a new database is born as (V-Q4: the pattern is stampable).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum DatabaseKind {
    /// The yeomna pattern: schema applied at birth, content through verbs.
    Kg,
    /// Empty Postgres: full utility through the `sql` verb.
    Plain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SqlRequest {
    /// Target database. KG-pattern targets are refused at runtime.
    pub database: String,
    /// Full statement text, which also lands in the audit args verbatim.
    pub statement: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EmbedTextRequest {
    pub text: String,
    /// The embedder task name (retrieval, passage, and friends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphEmbedNeighborsRequest {
    pub graph: String,
    pub key: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphEmbedUpdateRequest {
    pub graph: String,
    /// What to refresh: everything, or only stale rows.
    #[serde(default)]
    pub scope: UpdateScope,
}

/// Refresh scope for embedding updates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum UpdateScope {
    #[default]
    Stale,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IngestRequest {
    pub path: String,
    pub graph: String,
    #[serde(default)]
    pub overwrite: bool,
    /// Embed the chunks as they land (spec 022).
    ///
    /// Off by default. The embedder is a separate service on one GPU, a
    /// graph without vectors is still a graph and still searchable by
    /// keyword, and an ingest that quietly needed a service the operator
    /// had not started would be a worse default than one that has to be
    /// asked. When it is asked for and the embedder is unreachable the run
    /// fails before writing a chunk, rather than leaving text in the store
    /// with no vectors beside it (spec 011 EC-4).
    #[serde(default)]
    pub embed: bool,
    /// The task to embed at, which becomes `embeddings.task` and is the
    /// corpus half of the pairing a query has to match (R26). Absent means
    /// `retrieval.passage`, which is what a corpus is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_task: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RetireRequest {
    pub graph: String,
    /// Retire under this path prefix, and only what the tree no longer
    /// has. An empty prefix is refused, since it would name the whole
    /// graph.
    pub prefix: String,
    /// The working tree to compare against (R23, spec 020). The
    /// captured shape had no path, which predates D3's ruling that
    /// retire removes nodes whose sources are gone: without a tree the
    /// verb would have to take a prefix on faith and could delete the
    /// graph's record of source that is still there. A tree that cannot
    /// be walked is a refusal rather than a licence to sweep.
    pub path: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DropScoped {
    pub graph: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DriftRequest {
    pub graph: String,
    /// The working tree to compare against.
    pub path: String,
}

fn default_limit() -> u32 {
    10
}

fn default_page() -> u32 {
    20
}

fn default_depth() -> u32 {
    20
}

/// The hard row cap on enumerating traversal forms, per the store PRD.
fn default_row_cap() -> u32 {
    10_000
}

/// Every wire name in the contract, in the R4 table's order.
///
/// Public because a caller that cannot enumerate the vocabulary
/// cannot discover it, and a copy of this list somewhere else would
/// be a second contract. `wire_name`'s exhaustive match forces an
/// edit there when a variant is added, and its comment points here.
pub const WIRE_NAMES: [&str; 42] = [
    "orient",
    "status",
    "health",
    "check",
    "stats",
    "codebase.stats",
    "query",
    "get",
    "list",
    "count",
    "recent",
    "insert",
    "update",
    "delete",
    "purge",
    "graph.traverse",
    "graph.neighbors",
    "graph.shortest-path",
    "graph.list",
    "graph.create",
    "graph.drop",
    "graph.materialize",
    "schema.apply",
    "schema.list",
    "schema.show",
    "schema.version",
    "database.list",
    "database.create",
    "database.drop",
    "sql",
    "embed.text",
    "graph-embed.embed",
    "graph-embed.neighbors",
    "graph-embed.update",
    "ingest",
    "codebase.ingest",
    "codebase.retire",
    "codebase.prune",
    "codebase.drift",
    "codebase.validate",
    "edge.assert",
    "edge.retract",
];

impl Verb {
    /// The wire name, exactly as the R4 table binds it. Also the
    /// `command` field of the envelope and the `verb` column of the
    /// audit log.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Verb::Orient(_) => "orient",
            Verb::Status(_) => "status",
            Verb::Health(_) => "health",
            Verb::Check(_) => "check",
            Verb::Stats(_) => "stats",
            Verb::CodebaseStats(_) => "codebase.stats",
            Verb::Query(_) => "query",
            Verb::Get(_) => "get",
            Verb::List(_) => "list",
            Verb::Count(_) => "count",
            Verb::Recent(_) => "recent",
            Verb::Insert(_) => "insert",
            Verb::Update(_) => "update",
            Verb::Delete(_) => "delete",
            Verb::Purge(_) => "purge",
            Verb::EdgeAssert(_) => "edge.assert",
            Verb::EdgeRetract(_) => "edge.retract",
            Verb::GraphTraverse(_) => "graph.traverse",
            Verb::GraphNeighbors(_) => "graph.neighbors",
            Verb::GraphShortestPath(_) => "graph.shortest-path",
            Verb::GraphList(_) => "graph.list",
            Verb::GraphCreate(_) => "graph.create",
            Verb::GraphDrop(_) => "graph.drop",
            Verb::GraphMaterialize(_) => "graph.materialize",
            Verb::SchemaApply(_) => "schema.apply",
            Verb::SchemaList(_) => "schema.list",
            Verb::SchemaShow(_) => "schema.show",
            Verb::SchemaVersion(_) => "schema.version",
            Verb::DatabaseList(_) => "database.list",
            Verb::DatabaseCreate(_) => "database.create",
            Verb::DatabaseDrop(_) => "database.drop",
            Verb::Sql(_) => "sql",
            Verb::EmbedText(_) => "embed.text",
            Verb::GraphEmbedEmbed(_) => "graph-embed.embed",
            Verb::GraphEmbedNeighbors(_) => "graph-embed.neighbors",
            Verb::GraphEmbedUpdate(_) => "graph-embed.update",
            Verb::Ingest(_) => "ingest",
            Verb::CodebaseIngest(_) => "codebase.ingest",
            Verb::CodebaseRetire(_) => "codebase.retire",
            Verb::CodebasePrune(_) => "codebase.prune",
            Verb::CodebaseDrift(_) => "codebase.drift",
            Verb::CodebaseValidate(_) => "codebase.validate",
        }
    }
}
