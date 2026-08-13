//! The Yeomna store: schema and (in spec 009) the `IngestSink`
//! implementation, over the sealed Postgres cluster.
//!
//! Built per `docs/specs/008-store-schema/spec.md` under rulings R1
//! (`graph_id` column) and R2 (`full_page_writes` off on CoW). The schema
//! ships as embedded SQL and applies idempotently. No ORM, no pooling yet,
//! `tokio-postgres` over the Unix socket only: the store binds no network
//! listener, and neither does anything talking to it.

use tokio_postgres::{Client, NoTls};

/// The schema, embedded. Every statement tolerates re-application.
pub const SCHEMA_SQL: &str = include_str!("../schema.sql");

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Database error.
    #[error("database error: {0}")]
    Db(#[from] tokio_postgres::Error),
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
