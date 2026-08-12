//! The Yeomna CLI: the captured surface with every handler a hole.
//!
//! Captured from the reference CLI per spec 007. The clap tree, argument
//! shapes, and help text are the kept contract of the 32 verbs plus the
//! ingestion operations. Every command answers with the hole envelope
//! naming what it awaits, so this binary is the executable holes ledger:
//! as verbs land, holes become behavior.

mod defs;
mod output;

use std::process::ExitCode;

use clap::Parser;
use defs::*;

/// Yeomna: a sealed appliance turning a firm's documents and code into
/// queryable context, served over a single audited verb surface.
#[derive(Parser)]
#[command(name = "yeomna", version, about)]
pub struct Cli {
    /// Target graph name (overrides config/env).
    #[arg(long = "database", alias = "db", global = true)]
    database: Option<String>,

    /// GPU device index for embedding commands.
    #[arg(short = 'g', long = "gpu", global = true)]
    gpu: Option<u32>,

    #[command(subcommand)]
    command: Commands,
}

/// The hole envelope: one shape, stdout, nonzero exit. Holes render as
/// JSON regardless of any format flag, which the help text states.
fn hole(name: &str, awaits: &str) -> ExitCode {
    let envelope = serde_json::json!({
        "success": false,
        "hole": true,
        "error": format!("hole: {name} awaits {awaits}"),
    });
    println!("{envelope}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status { .. } => hole("status", "the verb layer and the store"),
        Commands::Orient { .. } => hole("orient", "the verb layer and the store"),
        Commands::Extract { .. } => hole(
            "extract",
            "the ingest orchestrator and the extraction backend",
        ),
        Commands::Ingest { .. } => hole(
            "ingest",
            "the ingest orchestrator and the extraction backend",
        ),
        Commands::Daemon { .. } => hole("daemon", "the daemon"),
        Commands::Db(cmd) => match cmd {
            DbCmd::Query { .. } => hole("db query", "the verb layer and the store"),
            DbCmd::List { .. } => hole("db list", "the verb layer and the store"),
            DbCmd::Stats { .. } => hole("db stats", "the verb layer and the store"),
            DbCmd::Recent { .. } => hole("db recent", "the verb layer and the store"),
            DbCmd::Health { .. } => hole("db health", "the verb layer and the store"),
            DbCmd::Check { .. } => hole("db check", "the verb layer and the store"),
            DbCmd::Purge { .. } => hole("db purge", "the verb layer and the store"),
            DbCmd::Create { .. } => hole("db create", "the verb layer and the store"),
            DbCmd::Delete { .. } => hole("db delete", "the verb layer and the store"),
            DbCmd::Collections { .. } => hole("db collections", "the verb layer and the store"),
            DbCmd::Databases { .. } => hole("db databases", "the verb layer and the store"),
            DbCmd::CreateDatabase { .. } => {
                hole("db create-database", "the verb layer and the store")
            }
            DbCmd::Truncate { .. } => hole("db truncate", "the verb layer and the store"),
            DbCmd::DropCollection { .. } => {
                hole("db drop-collection", "the verb layer and the store")
            }
            DbCmd::Count { .. } => hole("db count", "the verb layer and the store"),
            DbCmd::Get { .. } => hole("db get", "the verb layer and the store"),
            DbCmd::Insert { .. } => hole("db insert", "the verb layer and the store"),
            DbCmd::Update { .. } => hole("db update", "the verb layer and the store"),
            DbCmd::Export { .. } => hole("db export", "the verb layer and the store"),
            DbCmd::CreateIndex { .. } => hole("db create-index", "the verb layer and the store"),
            DbCmd::IndexStatus { .. } => hole("db index-status", "the verb layer and the store"),
            DbCmd::Graph(sub) => match sub {
                DbGraphCmd::Create { .. } => {
                    hole("db graph create", "the verb layer and the store")
                }
                DbGraphCmd::List { .. } => hole("db graph list", "the verb layer and the store"),
                DbGraphCmd::Drop { .. } => hole("db graph drop", "the verb layer and the store"),
                DbGraphCmd::Traverse { .. } => {
                    hole("db graph traverse", "the verb layer and the store")
                }
                DbGraphCmd::ShortestPath { .. } => {
                    hole("db graph shortest-path", "the verb layer and the store")
                }
                DbGraphCmd::Neighbors { .. } => {
                    hole("db graph neighbors", "the verb layer and the store")
                }
                DbGraphCmd::Materialize { .. } => {
                    hole("db graph materialize", "the verb layer and the store")
                }
            },
            DbCmd::Schema(sub) => match sub {
                DbSchemaCmd::Init { .. } => hole("db schema init", "the verb layer and the store"),
                DbSchemaCmd::List { .. } => hole("db schema list", "the verb layer and the store"),
                DbSchemaCmd::Show { .. } => hole("db schema show", "the verb layer and the store"),
                DbSchemaCmd::Version { .. } => {
                    hole("db schema version", "the verb layer and the store")
                }
            },
        },
        Commands::Embed(cmd) => match cmd {
            EmbedCmd::Text { .. } => hole("embed text", "the embedder backend"),
            EmbedCmd::Service(sub) => match sub {
                EmbedServiceCmd::Status => hole("embed service status", "the embedder backend"),
                EmbedServiceCmd::Start { .. } => {
                    hole("embed service start", "the embedder backend")
                }
                EmbedServiceCmd::Stop => hole("embed service stop", "the embedder backend"),
            },
            EmbedCmd::Gpu(sub) => match sub {
                EmbedGpuCmd::Status => hole("embed gpu status", "the embedder backend"),
                EmbedGpuCmd::List => hole("embed gpu list", "the embedder backend"),
            },
        },
        Commands::Codebase(cmd) => match cmd {
            CodebaseCmd::Ingest { .. } => {
                hole("codebase ingest", "the ingest orchestrator and the store")
            }
            CodebaseCmd::Update { .. } => {
                hole("codebase update", "the ingest orchestrator and the store")
            }
            CodebaseCmd::Stats => hole("codebase stats", "the ingest orchestrator and the store"),
            CodebaseCmd::Validate => {
                hole("codebase validate", "the ingest orchestrator and the store")
            }
            CodebaseCmd::PruneOrphans { .. } => hole(
                "codebase prune-orphans",
                "the ingest orchestrator and the store",
            ),
            CodebaseCmd::Drift { .. } => {
                hole("codebase drift", "the ingest orchestrator and the store")
            }
            CodebaseCmd::Retire { .. } => {
                hole("codebase retire", "the ingest orchestrator and the store")
            }
        },
        Commands::GraphEmbed(cmd) => match cmd {
            GraphEmbedCmd::Embed { .. } => hole(
                "graph-embed embed",
                "the graph-embed era (store plus trained embeddings)",
            ),
            GraphEmbedCmd::Neighbors { .. } => hole(
                "graph-embed neighbors",
                "the graph-embed era (store plus trained embeddings)",
            ),
            GraphEmbedCmd::Update { .. } => hole(
                "graph-embed update",
                "the graph-embed era (store plus trained embeddings)",
            ),
        },
        Commands::Schema(cmd) => match cmd {
            SchemaCmd::Apply { .. } => hole("schema apply", "the schema manager"),
        },
        Commands::Tools(cmd) => match cmd {
            ToolsCmd::Status { .. } => hole("tools status", "the analyzer toolchain manager"),
            ToolsCmd::Install { .. } => hole("tools install", "the analyzer toolchain manager"),
        },
    }
}
