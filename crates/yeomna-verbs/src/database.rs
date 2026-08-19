//! The three database lifecycle verbs, per spec 013 and V-Q4.
//!
//! Lifecycle runs as `yeomna_provision` (R13): CREATEDB, NOLOGIN, no
//! grants on any KG table, reached only by `SET ROLE` from the app role.
//! The kg stamp is `CREATE DATABASE ... TEMPLATE yeomna_template` (R13a),
//! because pgvector is not a trusted extension, so schema application in
//! a fresh database would need the superuser this role must never be.
//!
//! Database names cannot be bound as parameters, so they are validated
//! against a strict identifier shape and then quoted. The validation is
//! the gate, the quoting is belt and braces.

use serde_json::{Value, json};

use crate::error::VerbError;
use crate::execute::Session;
use crate::verb::{DatabaseCreateRequest, DatabaseKind, DropRequest};

/// The template every kg-pattern database is stamped from.
const TEMPLATE: &str = "yeomna_template";

fn db(e: tokio_postgres::Error) -> VerbError {
    VerbError::Internal(format!("store error: {e}"))
}

/// A database name this verb layer will speak: lowercase identifier,
/// nothing else. Postgres allows more, and the verb layer does not,
/// because the name is the one thing here that cannot be a bind
/// parameter.
fn check_name(name: &str) -> Result<(), VerbError> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(VerbError::InvalidArgs(format!(
            "database name {name:?} must be a lowercase identifier (a-z, 0-9, _), at most 63 bytes"
        )));
    }
    if name.starts_with("pg_") {
        return Err(VerbError::InvalidArgs(
            "names beginning pg_ are reserved by Postgres".into(),
        ));
    }
    Ok(())
}

/// `database.list`: what the cluster holds.
pub async fn list(s: &Session) -> Result<Value, VerbError> {
    let rows = s
        .client()
        .query(
            "SELECT d.datname, pg_get_userbyid(d.datdba), d.datistemplate,
                    CASE WHEN has_database_privilege(current_user, d.datname, 'CONNECT')
                         THEN pg_size_pretty(pg_database_size(d.datname)) END
             FROM pg_database d ORDER BY d.datname",
            &[],
        )
        .await
        .map_err(db)?;
    Ok(json!({
        "databases": rows.iter().map(|r| json!({
            "name": r.get::<_, String>(0),
            "owner": r.get::<_, String>(1),
            "is_template": r.get::<_, bool>(2),
            "size": r.get::<_, Option<String>>(3),
        })).collect::<Vec<_>>(),
    }))
}

/// `database.create`: stamp the yeomna pattern, or an empty database.
pub async fn create(s: &Session, r: &DatabaseCreateRequest) -> Result<Value, VerbError> {
    check_name(&r.name)?;
    let sql = match r.kind {
        DatabaseKind::Kg => format!("CREATE DATABASE \"{}\" TEMPLATE {TEMPLATE}", r.name),
        DatabaseKind::Plain => format!("CREATE DATABASE \"{}\"", r.name),
    };
    s.as_provision(&sql)
        .await
        .map_err(|e| match e.as_db_error() {
            Some(d) if d.code().code() == "42P04" => {
                VerbError::InvalidArgs(format!("database {:?} already exists", r.name))
            }
            // EC-4: a half-provisioned cluster names the missing step.
            Some(d) if d.message().contains(TEMPLATE) => VerbError::Internal(format!(
                "the {TEMPLATE} database is missing, run the provisioning step: {}",
                d.message()
            )),
            _ => db(e),
        })?;
    Ok(json!({
        "database": r.name,
        "kind": match r.kind { DatabaseKind::Kg => "kg", DatabaseKind::Plain => "plain" },
        "created": true,
    }))
}

/// `database.drop`: the customer's call, with the appliance's own
/// foundations protected by name and the primary protected by mechanism,
/// since the provision role does not own it.
pub async fn drop(s: &Session, r: &DropRequest) -> Result<Value, VerbError> {
    check_name(&r.name)?;
    if !r.force {
        return Err(VerbError::Denied(format!(
            "dropping database {:?} is unrecoverable, pass force to acknowledge",
            r.name
        )));
    }
    // Refused by name before any SQL: the session's own database, the
    // maintenance database, and any template.
    let current: String = s
        .client()
        .query_one("SELECT current_database()", &[])
        .await
        .map_err(db)?
        .get(0);
    if r.name == current || r.name == "postgres" {
        return Err(VerbError::Denied(format!(
            "{:?} is the appliance's own footing and this verb will not drop it",
            r.name
        )));
    }
    let is_template: Option<bool> = s
        .client()
        .query_opt(
            "SELECT datistemplate FROM pg_database WHERE datname = $1",
            &[&r.name],
        )
        .await
        .map_err(db)?
        .map(|row| row.get(0));
    match is_template {
        None => {
            return Err(VerbError::NotFound(format!(
                "no database named {:?}",
                r.name
            )));
        }
        Some(true) => {
            return Err(VerbError::Denied(format!(
                "{:?} is a template and this verb will not drop it",
                r.name
            )));
        }
        Some(false) => {}
    }
    s.as_provision(&format!("DROP DATABASE \"{}\"", r.name))
        .await
        .map_err(|e| match e.as_db_error() {
            // Not the owner: Postgres protecting what the provision role
            // did not stamp, the primary above all (spec 013).
            Some(d) if d.code().code() == "42501" => VerbError::Denied(format!(
                "the provision role does not own {:?}: {}",
                r.name,
                d.message()
            )),
            // EC-3: live connections.
            Some(d) if d.code().code() == "55006" => {
                VerbError::Denied(format!("{:?} is in use: {}", r.name, d.message()))
            }
            _ => db(e),
        })?;
    Ok(json!({ "database": r.name, "dropped": true }))
}
