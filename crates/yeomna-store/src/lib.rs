//! The Yeomna store: schema and (in spec 009) the `IngestSink`
//! implementation, over the sealed Postgres cluster.
//!
//! Built per `docs/specs/008-store-schema/spec.md` under rulings R1
//! (`graph_id` column) and R2 (`full_page_writes` off on CoW). The schema
//! ships as embedded SQL and applies idempotently. No ORM, no pooling yet,
//! `tokio-postgres` over the Unix socket only: the store binds no network
//! listener, and neither does anything talking to it.

use tokio_postgres::{Client, NoTls};

mod sink;
pub use sink::PgSink;

/// The schema, embedded. Every statement tolerates re-application.
pub const SCHEMA_SQL: &str = include_str!("../schema.sql");

/// The schema this binary ships (spec 012).
///
/// Compiled in, with no marker stored in the database. A stored marker
/// exists to detect drift between a database and the binary, and drift is
/// not a state this appliance allows: a schema change costs a drop and a
/// re-ingest, because the graph is a rebuildable index. Recording a
/// version in the database would invite the comparison that invites the
/// migration this project declines to own.
///
/// Bump it when `schema.sql` changes shape.
///
/// 1.1.0 (spec 014, R18): the relation vocabulary moved from the edges
/// parent to per-partition CHECKs, closed on declared and structural,
/// identifier-shaped on asserted. A 1.0.0 database refuses asserted
/// relations outside the old closed list, which is drift, so the bump
/// costs the drop and re-ingest the header promises.
///
/// 1.2.0 (spec 015, R19a): the declared partition opens to identifier
/// shape too, hyphen admitted on both open partitions, because declared
/// edges carry the source's own words and the first corpus speaks
/// kebab. Structural stays closed. Same cost, same path: re-stamp the
/// template, recreate, re-ingest.
pub const SCHEMA_VERSION: &str = "1.2.0";

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Database error.
    #[error("database error: {0}")]
    Db(#[from] tokio_postgres::Error),
    /// A container name outside the closed profile vocabulary: a caller
    /// bug, never a counted per-document error.
    #[error("unknown container: {0}")]
    UnknownContainer(String),
    /// A removal field outside the known parent-key set. Refused rather
    /// than guessed, because a guess deletes the wrong rows silently.
    #[error("unknown removal field: {0}")]
    UnknownRemovalField(String),
}

/// Connect over a Unix socket directory as the given role.
///
/// Peer authentication applies: the connecting OS user must map to `role`
/// in `pg_ident.conf`. There is no password because there is no network.
pub async fn connect(
    socket_dir: &str,
    port: u16,
    role: &str,
    database: &str,
) -> Result<Client, StoreError> {
    let (client, connection) = tokio_postgres::Config::new()
        .host_path(socket_dir)
        .port(port)
        .user(role)
        .dbname(database)
        .connect_timeout(std::time::Duration::from_secs(10))
        .connect(NoTls)
        .await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::warn!(%e, "postgres connection task ended");
        }
    });
    Ok(client)
}

/// Apply the schema idempotently. Run as the owner role: DDL is not the
/// app role's to perform.
///
/// Safe under concurrency: an advisory lock serializes appliers, since
/// concurrent IF NOT EXISTS DDL races in the catalogs even when each
/// statement alone is idempotent.
pub async fn apply_schema(client: &Client) -> Result<(), StoreError> {
    /// Arbitrary but stable: the advisory key for schema application.
    const SCHEMA_LOCK: i64 = 0x59454f4d; // "YEOM"
    // Bound the wait: a stalled applier must surface as an error, not as
    // every other applier hanging forever.
    client.batch_execute("SET lock_timeout = '30s'").await?;
    client
        .execute("SELECT pg_advisory_lock($1)", &[&SCHEMA_LOCK])
        .await?;
    let result = client.batch_execute(SCHEMA_SQL).await;
    if let Err(e) = client
        .execute("SELECT pg_advisory_unlock($1)", &[&SCHEMA_LOCK])
        .await
    {
        tracing::warn!(%e, "advisory unlock failed, lock releases on disconnect");
    }
    result?;
    Ok(())
}
