//! The captured surface: clap trees and help text from the reference,
//! per spec 007. Definitions only. Every handler is a hole in main.rs.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// System status for workspace discovery.
    Status {
        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,

        /// Verbose output.
        #[arg(short = 'V', long)]
        verbose: bool,
    },

    /// Metadata-first context orientation for a database.
    Orient {
        /// Collection to orient on.
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Extract text from documents (PDF, LaTeX, etc).
    Extract {
        /// File path to extract from.
        file: PathBuf,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,

        /// Output file path.
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },

    /// Ingest documents into the knowledge base.
    Ingest {
        /// Input file paths to ingest.
        inputs: Vec<String>,

        /// Custom document ID (single input only).
        #[arg(long)]
        id: Option<String>,

        /// Run in batch mode.
        #[arg(short = 'b', long)]
        batch: bool,

        /// Resume a previously interrupted batch.
        #[arg(short = 'r', long, conflicts_with = "reset")]
        resume: bool,

        /// Custom metadata as JSON.
        #[arg(short = 'm', long)]
        metadata: Option<String>,

        /// Embedding task type (e.g. "code" for Jina Code LoRA).
        #[arg(short = 't', long)]
        task: Option<String>,

        /// Collection profile name (must be defined in the runtime schema).
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Force re-processing of existing documents.
        #[arg(short = 'f', long)]
        force: bool,

        /// Reset batch state (clear previous checkpoint).
        #[arg(long, conflicts_with = "resume")]
        reset: bool,

        /// Maximum concurrent items (overrides config, must be >= 1).
        #[arg(long)]
        concurrency: Option<NonZeroUsize>,
    },

    /// Database operations — query, CRUD, indexes, and graph traversal.
    #[command(subcommand)]
    Db(DbCmd),

    /// Embedding generation and service management.
    #[command(subcommand)]
    Embed(EmbedCmd),

    /// Code ingestion and graph operations.
    #[command(subcommand)]
    Codebase(CodebaseCmd),

    /// Graph embedding operations — train and query structural embeddings.
    #[command(subcommand)]
    GraphEmbed(GraphEmbedCmd),

    /// Declarative schema operations — apply YAML schema files at bootstrap.
    ///
    /// Distinct from `db schema {init,list,show,version}` which inspects
    /// the live `runtime schema` collection. See `docs/declarative-schema.md`.
    #[command(subcommand)]
    Schema(SchemaCmd),

    /// External analyzer inventory and health (rust-analyzer, gopls).
    #[command(subcommand)]
    Tools(ToolsCmd),

    /// Start the Yeomna daemon (Unix socket query server, optional LAN MCP endpoint).
    Daemon {
        /// Socket path (default: /run/yeomna/yeomna.sock).
        #[arg(long)]
        socket: Option<String>,

        /// Serve the MCP endpoint on this address (loopback or RFC1918
        /// only, e.g. 192.168.0.10:8088). Requires --mcp-token-file.
        #[arg(long, env = "Yeomna_MCP_BIND")]
        mcp_bind: Option<String>,

        /// Databases the MCP endpoint serves in addition to the
        /// configured default (comma-separated). Anything not listed is
        /// refused — remote reads are scoped, writes stay ACL-gated.
        #[arg(long, env = "Yeomna_MCP_DBS", value_delimiter = ',')]
        mcp_dbs: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DbCmd {
    /// Semantic search across the knowledge base.
    Query {
        /// Search text. Required — there is no interactive mode, and omitting
        /// it exits non-zero without searching.
        search_text: Option<String>,

        /// Maximum results to return.
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: u32,

        /// Collection profile to search.
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Enable hybrid search (vector + keyword).
        #[arg(short = 'H', long)]
        hybrid: bool,

        /// Enable re-ranking of results. NOT AVAILABLE: the cross-encoder this
        /// needs does not ship with the CLI, so passing it exits non-zero
        /// without searching. Use `--hybrid` and/or `--structural` instead.
        #[arg(short = 'R', long)]
        rerank: bool,

        /// Enable structural graph ranking.
        #[arg(short = 'S', long)]
        structural: bool,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,

        /// Verbose output.
        #[arg(short = 'V', long)]
        verbose: bool,
    },

    /// List documents in a collection.
    List {
        /// Collection profile.
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Maximum results.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,

        /// Filter by paper ID.
        #[arg(short = 'p', long)]
        paper: Option<String>,
    },

    /// Show database statistics.
    Stats {
        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Show recently ingested papers.
    Recent {
        /// Maximum results.
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: u32,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Check data integrity and health.
    Health {
        /// Verbose output.
        #[arg(short = 'V', long)]
        verbose: bool,
    },

    /// Check if a document exists.
    Check {
        /// Document ID to check.
        document_id: String,
    },

    /// Remove a document and its related data.
    Purge {
        /// Document ID to purge.
        document_id: String,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// Create a new collection.
    Create {
        /// Collection name.
        name: String,

        /// Collection type (document, edge).
        #[arg(short = 't', long, default_value = "document")]
        r#type: String,
    },

    /// Delete a document from a collection.
    Delete {
        /// Collection name.
        collection: String,

        /// Document key.
        key: String,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// List all collections in the database.
    Collections {
        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// List all databases.
    Databases {
        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Create a new database.
    CreateDatabase {
        /// Database name.
        name: String,
    },

    // NOTE: there is deliberately no `drop-database` command. Dropping a whole
    // database is a console-only ritual — the data is sacrosanct, the blast
    // radius is total and irreversible, and no agent workflow needs it. Use the
    // store console directly. See the reference's issue #118.
    /// Empty a collection in place (keeps the collection and its indexes).
    Truncate {
        /// Collection name.
        collection: String,

        /// Confirm the operation. Required — this permanently deletes all
        /// documents in the collection.
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// Drop a single collection (documents, indexes, and the collection itself).
    DropCollection {
        /// Collection name.
        collection: String,

        /// Confirm the operation. Required — this permanently removes the
        /// collection.
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// Count documents in a collection.
    Count {
        /// Collection name.
        collection: String,
    },

    /// Get a single document by key.
    Get {
        /// Collection name.
        collection: String,

        /// Document key.
        key: String,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Insert documents into a collection.
    Insert {
        /// Collection name.
        collection: String,

        /// JSON document(s) to insert (reads from stdin if omitted).
        #[arg(long)]
        data: Option<String>,

        /// Input file path.
        #[arg(short = 'i', long)]
        input: Option<PathBuf>,
    },

    /// Update a document in a collection.
    Update {
        /// Collection name.
        collection: String,

        /// Document key.
        key: String,

        /// JSON fields to update.
        #[arg(long)]
        data: Option<String>,
    },

    /// Export a collection to file.
    Export {
        /// Collection name.
        collection: String,

        /// Output file path.
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "jsonl")]
        format: String,

        /// Maximum documents to export.
        #[arg(short = 'n', long)]
        limit: Option<u32>,
    },

    /// Create a vector index on a collection.
    CreateIndex {
        /// Collection name.
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Vector dimension.
        #[arg(long)]
        dimension: Option<u32>,

        /// Distance metric (cosine, euclidean, dotproduct).
        #[arg(long)]
        metric: Option<String>,
    },

    /// Show vector index status.
    IndexStatus {
        /// Collection name.
        #[arg(short = 'c', long)]
        collection: Option<String>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Graph operations.
    #[command(subcommand)]
    Graph(DbGraphCmd),

    /// Schema management — define and inspect database ontologies.
    #[command(subcommand)]
    Schema(DbSchemaCmd),
}

#[derive(Debug, Subcommand)]
pub enum DbGraphCmd {
    /// Create a named graph.
    Create {
        /// Graph name.
        name: String,

        /// Edge definitions as JSON.
        #[arg(long)]
        edge_definitions: Option<String>,
    },

    /// List all named graphs.
    List {
        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Drop a named graph.
    Drop {
        /// Graph name.
        name: String,

        /// Also drop associated collections.
        #[arg(long)]
        drop_collections: bool,

        /// Skip confirmation.
        #[arg(short = 'y', long)]
        force: bool,
    },

    /// Traverse the graph from a starting vertex.
    Traverse {
        /// Starting vertex ID.
        start: String,

        /// Traversal direction (outbound, inbound, any).
        #[arg(short = 'd', long, default_value = "outbound")]
        direction: String,

        /// Minimum depth.
        #[arg(long, default_value_t = 1)]
        min_depth: u32,

        /// Maximum depth.
        #[arg(long, default_value_t = 3)]
        max_depth: u32,

        /// Graph name.
        #[arg(long)]
        graph: Option<String>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Find the shortest path between two vertices.
    ShortestPath {
        /// Source vertex ID.
        source: String,

        /// Target vertex ID.
        target: String,

        /// Graph name.
        #[arg(long)]
        graph: Option<String>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Find neighbors of a vertex.
    Neighbors {
        /// Vertex ID.
        vertex: String,

        /// Traversal direction (outbound, inbound, any).
        #[arg(short = 'd', long, default_value = "any")]
        direction: String,

        /// Maximum results.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,

        /// Graph name.
        #[arg(long)]
        graph: Option<String>,

        /// Output format (json, jsonl, table).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Materialize edges from implicit cross-reference fields.
    Materialize {
        /// Filter to a single edge definition name.
        #[arg(short = 'e', long)]
        edge: Option<String>,

        /// Preview mode — count edges without inserting.
        #[arg(long)]
        dry_run: bool,

        /// Also create named graphs via the Gharial API.
        #[arg(short = 'r', long)]
        register: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DbSchemaCmd {
    /// Initialize the runtime schema collection with a seed ontology.
    Init {
        /// Seed name. Only "empty" is currently accepted — initializes a
        /// `runtime schema` collection with metadata only and no edge definitions.
        #[arg(short = 's', long)]
        seed: String,
    },

    /// List all edge definitions and named graphs in the schema.
    List {},

    /// Show a single edge definition or named graph by name.
    Show {
        /// Name of the edge definition or named graph.
        name: String,
    },

    /// Show schema version, checksum, and metadata.
    Version {},
}

#[derive(Debug, Subcommand)]
pub enum EmbedCmd {
    /// Generate an embedding for the given text.
    Text {
        /// Text to embed.
        text: String,

        /// Output format (json, raw).
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// Embedding service management.
    #[command(subcommand)]
    Service(EmbedServiceCmd),

    /// GPU device management.
    #[command(subcommand)]
    Gpu(EmbedGpuCmd),
}

#[derive(Debug, Subcommand)]
pub enum EmbedServiceCmd {
    /// Show embedding service status.
    Status,

    /// Start the embedding service.
    Start {
        /// Run in foreground (don't daemonize).
        #[arg(long)]
        foreground: bool,
    },

    /// Stop the embedding service.
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum EmbedGpuCmd {
    /// Show GPU status and memory usage.
    Status,

    /// List available GPU devices.
    List,
}

#[derive(Debug, Subcommand)]
pub enum CodebaseCmd {
    /// Ingest source code into the knowledge graph.
    Ingest {
        /// Path to file or directory to ingest.
        path: PathBuf,

        /// Programming language override (auto-detected if omitted).
        #[arg(short = 'l', long)]
        language: Option<String>,

        /// Run in batch mode.
        #[arg(short = 'b', long)]
        batch: bool,

        /// Comma-separated extensions to embed without a parser (e.g.
        /// `wgsl,vert`). Files with these extensions are chunked by size and
        /// embedded as features — no symbol/edge extraction. Their file nodes
        /// are merged (existing fields preserved), not overwritten.
        #[arg(long = "unparsed-ext", value_delimiter = ',')]
        unparsed_ext: Vec<String>,

        /// Path to `compile_commands.json` (or its containing directory) for
        /// compiler-grade C/C++/CUDA include, define, standard, and target
        /// configuration. When omitted, source ancestors and `build/` are
        /// searched automatically.
        #[arg(long = "compile-commands")]
        compile_commands: Option<PathBuf>,

        /// Re-ingest each file even if its change-detection digest is
        /// unchanged. This rebuilds the node's symbols, chunks, and embeddings
        /// in place — it does NOT drop the file node or its inbound edges, so
        /// authored bridge edges survive (unlike `db purge`). Use it to refresh
        /// a node whose stored view has drifted from the source — in particular
        /// after an edit that touched only bodies, signatures, or comments,
        /// which a name-keyed `symbol_hash` cannot see and which
        /// `codebase drift` reports as `changed`. What `symbol_hash` covers
        /// depends on the node's `analysis_tier` — see `codebase drift --help`;
        /// the name-only case is Python and Rust at tier `semantic`.
        ///
        /// Pass the ORIGINAL ingest root, not a narrower path. Keys are
        /// derived relative to the path given (a file bases at its parent), so
        /// re-ingesting a single file or subdirectory writes duplicate nodes
        /// under re-based keys, purges nothing, and repairs nothing.
        ///
        /// If a rebuild drops a symbol that another file points at, those
        /// inbound edges are reported as `dangling_inbound_edges` — not
        /// deleted, since each records a real dependency. Re-resolve them by
        /// re-running `codebase ingest --force <the same ingest root>` (plain
        /// re-ingest skips the dependents, whose own `symbol_hash` did not
        /// change), or run `yeomna codebase prune-orphans` to drop them; until
        /// then `codebase validate` will flag them.
        ///
        /// This never permits an analyzer-fidelity downgrade by itself, so a
        /// file whose stored analysis came from a richer analyzer than the one
        /// available now is still skipped — pass `--allow-analysis-downgrade`
        /// as well to refresh it.
        ///
        /// One exception, and it is the recovery path for #193: a `.go` node
        /// whose stored `semantic` tier came from the old gopls stamp IS
        /// rewritten to `structural`, without `--allow-analysis-downgrade`.
        /// Go has no per-file semantic analyzer, so that stamp described a
        /// fidelity the node's own digest never had, and the gopls phase
        /// re-supplies the semantic symbols and edges later in the same run.
        #[arg(short = 'f', long = "force", alias = "no-skip")]
        force: bool,

        /// Permit a lower-fidelity analyzer to replace previously stored
        /// semantic artifacts. This is separate from `--force` so a temporary
        /// analyzer outage cannot silently degrade the graph.
        #[arg(long = "allow-analysis-downgrade")]
        allow_analysis_downgrade: bool,
    },

    /// Update an existing code graph node.
    Update {
        /// Path to file or directory to update.
        path: PathBuf,
    },

    /// Show code ingestion statistics.
    Stats,

    /// Validate codebase graph invariants (ontology spec §10).
    Validate,

    /// Remove orphaned symbols, chunks, embeddings, and dangling edges.
    ///
    /// Sweeps child records whose owning file node is already gone. To retire a
    /// file node whose *source file* was deleted, use `codebase retire`.
    PruneOrphans {
        /// Report what would be deleted without modifying the graph.
        #[arg(long)]
        dry_run: bool,
    },

    /// Compare the graph against the source tree it describes (read-only).
    ///
    /// `codebase validate` checks only internal consistency and cannot see any
    /// of this.
    ///
    /// Buckets: `stale` (a file node with no counterpart under this root),
    /// `uningested` (a source file with no node), `changed` (a matched file
    /// whose content differs from what was ingested), and `unhandled` (files
    /// under the root ingest has no handler for, with a reason for each).
    ///
    /// `changed.unverifiable` counts matched files that could not be compared at
    /// all, because they were ingested before `content_hash` existed or are no
    /// longer readable as text.
    ///
    /// `clean` is true only when stale, uningested, changed and
    /// `changed.unverifiable` are all zero. `unhandled` does not gate it, since
    /// every repository contains files no analyzer handles.
    ///
    /// `changed` exists because drift compares full content while incremental
    /// ingest compares `symbol_hash`, whose meaning depends on the node's
    /// `analysis_tier`. At tier `semantic`, Python and Rust hash symbol *names*
    /// only, so an edited body, signature or comment leaves it identical and a
    /// plain `codebase ingest` skips the file while its stored chunks go stale.
    /// Refresh those with `codebase ingest --force`.
    ///
    /// The other tiers are stricter and mostly self-correct: tier `structural`
    /// and C++ at tier `semantic` hash the serialized symbol list (line spans
    /// and metadata included), and tier `text` hashes full content. Check the
    /// tier rather than the extension. Go is in the `structural` group: it has
    /// no per-file semantic analyzer, and the semantic symbols and edges gopls
    /// contributes are recorded under their own keys rather than by restating
    /// the file's tier.
    ///
    /// One exception on graphs built before that changed: `.go` nodes ingested
    /// by an older Yeomna still read `semantic` even though their digest is
    /// tree-sitter's, and a plain re-ingest will not clear the stamp because
    /// the unchanged-digest skip fires first. `codebase ingest --force <the
    /// original ingest root>` rewrites them; until then, treat a `.go` node
    /// reading `semantic` as `structural`.
    ///
    /// `stale` is NOT "the source file was deleted". It is every node with no
    /// counterpart under the root you passed. Nodes belonging to another
    /// ingest root are excluded and counted separately as `other_roots`, so a
    /// database holding several trees no longer reports one tree's nodes as
    /// stale for another (#192).
    ///
    /// The exception is nodes ingested before Yeomna recorded `ingest_root`.
    /// Those cannot be attributed either way, so they are still compared and the
    /// stale ones are listed separately as `stale.unattributed_keys`, held out
    /// of `stale.keys` so a `--full` pipe into `codebase retire` cannot delete
    /// them unreviewed.
    ///
    /// `other_roots` reports the roots as well as the count, because a root
    /// *under* this one is usually a mis-rooted ingest of this same tree rather
    /// than a second graph: `codebase ingest` on a single file bases its keys at
    /// that file's parent.
    ///
    /// Pass the same discovery flags used at ingest time, and the same root —
    /// keys are relative to the ingest root, so a wrong root reports near-total
    /// drift in both directions rather than a small honest number.
    Drift {
        /// Ingest root the graph was built from.
        path: PathBuf,

        /// Programming language override (must match the ingest invocation).
        #[arg(short = 'l', long)]
        language: Option<String>,

        /// Extensions ingested without a parser (must match the ingest
        /// invocation), e.g. `wgsl,vert`.
        #[arg(long = "unparsed-ext", value_delimiter = ',')]
        unparsed_ext: Vec<String>,

        /// List every key instead of truncating. Use this to feed
        /// `codebase retire --from -`.
        ///
        /// `stale.keys` holds only nodes attributed to this ingest root, so it
        /// is what `retire` should be fed. Keys that could not be attributed are
        /// held out, in `stale.unattributed_keys`, for review — `retire` deletes
        /// each target's node, chunks, embeddings, symbols and incident edges,
        /// and those keys predate the attribution that would prove they belong
        /// to this tree.
        ///
        /// Re-ingesting a root attributes every node whose file still exists. A
        /// node whose file is already gone is never rediscovered, so no re-ingest
        /// can attribute it; that residue is pre-attribution backlog and only a
        /// reviewed retire clears it.
        #[arg(long)]
        full: bool,
    },

    /// Retire graph nodes whose source files are gone (complement of --force).
    ///
    /// Removes each target's file node, chunks, embeddings, symbols, and every
    /// codebase edge incident on the file or its symbols. Edges in other
    /// collections (authored bridges such as conformance verdicts) are reported
    /// separately and need `--yes`, since they are irreplaceable if the target
    /// list is wrong.
    ///
    /// Targets are always explicit — use `codebase drift` to discover them.
    Retire {
        /// File node key to retire. Repeatable.
        #[arg(long = "file")]
        files: Vec<String>,

        /// Read newline-separated keys from a file (`-` for stdin).
        /// Blank lines and `#` comments are ignored.
        #[arg(long = "from")]
        from: Option<PathBuf>,

        /// Report what would be removed without modifying the graph.
        #[arg(long)]
        dry_run: bool,

        /// Confirm removal of edges outside the codebase collections.
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum GraphEmbedCmd {
    /// Generate embedding for a specific node.
    Embed {
        /// Node ID to embed.
        node_id: String,
    },

    /// Find nearest neighbors of a node in embedding space.
    Neighbors {
        /// Node ID to query.
        node_id: String,

        /// Number of neighbors to return.
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: u32,
    },

    /// Update graph embeddings incrementally (forward pass only, no retraining).
    Update {
        /// Export embeddings to a different database after update.
        /// If omitted, exports to the current database.
        #[arg(long)]
        export_to: Option<String>,

        /// Checkpoint directory containing the trained model. Must be writable
        /// by both you and the `yeomna` training-service user (see `train`),
        /// since the service reads the graph IPC file written here.
        #[arg(long, default_value = "/tmp/yeomna-train")]
        checkpoint_dir: String,

        /// Embed only graph nodes whose destination documents do not yet have
        /// `structural_embedding`. Requires an inductive `hetero_sage` schema
        /// and checkpoint; existing embeddings are left untouched.
        #[arg(long)]
        new_nodes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum SchemaCmd {
    /// Apply a YAML schema file to the database.
    ///
    /// Bootstrap-only by default: refuses to run if the database
    /// already has user-data in any of the file's declared
    /// collections. Pass `--force` to override (dangerous).
    Apply {
        /// Path to the YAML schema file.
        file: PathBuf,

        /// Build the plan and print it without executing any writes.
        #[arg(long)]
        dry_run: bool,

        /// Skip the in-use guard. Allows applying to a database that
        /// already has data in declared collections; existing
        /// documents are overwritten by `_key`.
        #[arg(short = 'y', long)]
        force: bool,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum ToolsCmd {
    /// Report each analyzer's resolution and a live probe from a workspace.
    Status {
        /// Directory to probe FROM (the shim resolves per-directory). Defaults
        /// to the current directory — pass the ingest root you intend to use.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },

    /// Install a Yeomna-managed analyzer binary into the tools directory.
    Install {
        /// Which analyzer: rust-analyzer (GitHub release download) or
        /// gopls (built via `go install`; requires a Go toolchain).
        tool: String,

        /// Release tag (rust-analyzer, e.g. 2026-07-14) or module version
        /// (gopls, e.g. v0.21.0). Defaults to latest.
        #[arg(long)]
        version: Option<String>,

        /// Install a rust-analyzer asset even when the release carries no
        /// sha256 digest (default: refuse — no integrity check at all).
        #[arg(long)]
        allow_unverified: bool,
    },
}
