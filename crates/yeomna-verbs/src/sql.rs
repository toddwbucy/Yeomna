//! The scoped `sql` verb (spec 014, V-Q4 applied via R17 and R17a).
//!
//! Full utility against plain databases, structural refusal against the
//! kg pattern, and the provision role as the backstop: statements run as
//! `yeomna_provision` on a per-call connection to the target database,
//! which owns the plain databases it created and holds no grant on any
//! KG table (R13). The connection drops when the call ends, so unlike
//! the session's own escalation there is no reset to prove: the drop is
//! the reset (R17).
//!
//! The kg pattern is detected by asking the target what it is (R17a): a
//! catalog probe for the schema's signature tables. Structure decides
//! and provenance does not, so a database stamped by hand is protected
//! the same as one stamped by `database.create`, and detection errs
//! toward refusal.

use serde_json::{Value, json};
use tokio_postgres::SimpleQueryMessage;

use crate::database::check_name;
use crate::error::VerbError;
use crate::execute::Exec;
use crate::read::MAX_PAGE;
use crate::verb::SqlRequest;

/// The tables whose presence in schema `public` marks a kg-pattern
/// database. Any one of them is enough: a partial stamp is still not a
/// plain database, and refusal is the safe reading.
const SIGNATURE: [&str; 5] = ["graphs", "nodes", "edges", "node_log", "audit_log"];

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

/// Postgres's own answer, carried through the taxonomy: a missing grant
/// is a denial, anything else the statement did wrong is the caller's to
/// fix, with the SQLSTATE in the detail either way.
fn statement_error(e: tokio_postgres::Error) -> VerbError {
    match e.as_db_error() {
        Some(d) if d.code().code() == "42501" => {
            VerbError::Denied(format!("{}: {}", d.code().code(), d.message()))
        }
        Some(d) => VerbError::InvalidArgs(format!("{}: {}", d.code().code(), d.message())),
        None => db(e),
    }
}

/// `sql` (FR8). The gate order is cheapest first: names, then the
/// connection, then the structural probe, then the role, then the text.
pub async fn sql(
    s: &Exec<'_>,
    endpoint: Option<(&str, u16)>,
    r: &SqlRequest,
) -> Result<Value, VerbError> {
    check_name(&r.database)?;
    let current: String = s
        .client()
        .query_one("SELECT current_database()", &[])
        .await
        .map_err(db)?
        .get(0);
    if r.database == current || r.database == "yeomna" || r.database == "postgres" {
        return Err(VerbError::Denied(format!(
            "{:?} is the appliance's own footing, reached through verbs and never through sql",
            r.database
        )));
    }
    let is_template: Option<bool> = s
        .client()
        .query_opt(
            "SELECT datistemplate FROM pg_database WHERE datname = $1",
            &[&r.database],
        )
        .await
        .map_err(db)?
        .map(|row| row.get(0));
    match is_template {
        None => {
            return Err(VerbError::NotFound(format!(
                "no database named {:?}",
                r.database
            )));
        }
        Some(true) => {
            return Err(VerbError::Denied(format!(
                "{:?} is a template and sql does not touch templates",
                r.database
            )));
        }
        Some(false) => {}
    }
    let Some((dir, port)) = endpoint else {
        return Err(VerbError::Internal(
            "this session was built without an endpoint, so sql cannot reach another database"
                .into(),
        ));
    };

    // The per-call connection (R17). Dropped at the end of this function,
    // escalation and all, which is why no reset needs proving.
    let target = yeomna_store::connect(dir, port, "yeomna_app", &r.database)
        .await
        .map_err(|e| VerbError::Internal(format!("could not reach {:?}: {e}", r.database)))?;

    // R17a: ask the database what it is before running anything.
    let signature: Vec<String> = SIGNATURE.iter().map(|t| t.to_string()).collect();
    let marks: i64 = target
        .query_one(
            "SELECT count(*) FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
               AND c.relname = ANY($1)",
            &[&signature],
        )
        .await
        .map_err(db)?
        .get(0);
    if marks > 0 {
        return Err(VerbError::Denied(format!(
            "{:?} carries the kg pattern ({marks} signature tables), which is reached \
             through verbs and never through sql",
            r.database
        )));
    }

    target
        .batch_execute("SET ROLE yeomna_provision")
        .await
        .map_err(db)?;

    // simple_query because the statement text is the whole interface:
    // multiple statements are the utility, and there are no parameters
    // to bind.
    let messages = target
        .simple_query(&r.statement)
        .await
        .map_err(statement_error)?;

    let mut results: Vec<Value> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    let mut truncated = 0u64;
    for message in messages {
        match message {
            SimpleQueryMessage::RowDescription(desc) => {
                columns = desc.iter().map(|c| c.name().to_string()).collect();
                rows.clear();
                truncated = 0;
            }
            SimpleQueryMessage::Row(row) => {
                if rows.len() < MAX_PAGE as usize {
                    let mut out = Vec::with_capacity(row.len());
                    for i in 0..row.len() {
                        out.push(match row.get(i) {
                            Some(v) => json!(v),
                            None => Value::Null,
                        });
                    }
                    rows.push(Value::Array(out));
                } else {
                    truncated += 1;
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                results.push(json!({
                    "columns": std::mem::take(&mut columns),
                    "rows": std::mem::take(&mut rows),
                    "rows_affected": n,
                    "truncated_rows": truncated,
                    "limit": MAX_PAGE,
                }));
                truncated = 0;
            }
            _ => {}
        }
    }
    Ok(json!({
        "database": r.database,
        "role": "yeomna_provision",
        "results": results,
    }))
}
