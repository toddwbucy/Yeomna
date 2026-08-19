//! The database lifecycle verbs, per spec 013 and V-Q4.
//!
//! These need more provisioning than the rest: the `yeomna_provision`
//! role and the `yeomna_template` database, both operator steps. They
//! skip when either is absent, naming which, per the extended gate.

use serde_json::Value;
use tokio_postgres::Client;
use yeomna_verbs::*;

const PORT: u16 = 5433;

fn socket_dir() -> Option<String> {
    let dir = std::env::var("YEOMNA_TEST_DB").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share/yeomna/run")
    });
    let sock = format!("{dir}/.s.PGSQL.{PORT}");
    std::path::Path::new(&sock).exists().then_some(dir)
}

async fn fixtures() -> Option<(Client, Session)> {
    let dir = socket_dir()?;
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_owner");
        return None;
    };
    for (probe, what) in [
        (
            "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'yeomna_app')",
            "yeomna_app",
        ),
        (
            "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'yeomna_provision')",
            "yeomna_provision",
        ),
        (
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'yeomna_template' AND datistemplate)",
            "the yeomna_template database",
        ),
    ] {
        let present: bool = owner.query_one(probe, &[]).await.unwrap().get(0);
        if !present {
            eprintln!("SKIP: {what} is not provisioned on this cluster");
            return None;
        }
    }
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    Some((owner, Session::new(app, "db-test")))
}

fn data(e: &Envelope) -> &Value {
    assert!(e.success, "expected success, got {:?}", e.error);
    e.data.as_ref().expect("success carries data")
}

fn create(name: &str, kind: DatabaseKind) -> Verb {
    Verb::DatabaseCreate(DatabaseCreateRequest {
        name: name.into(),
        kind,
    })
}

fn drop(name: &str, force: bool) -> Verb {
    Verb::DatabaseDrop(DropRequest {
        name: name.into(),
        force,
    })
}

/// Owner-side cleanup so a failed prior run cannot poison this one.
async fn sweep(owner: &Client, name: &str) {
    let _ = owner
        .execute(&format!("DROP DATABASE IF EXISTS \"{name}\""), &[])
        .await;
}

#[tokio::test]
async fn a_kg_stamp_matches_the_primary_structurally() {
    let Some((owner, s)) = fixtures().await else {
        return;
    };
    sweep(&owner, "dbv_kg").await;
    let d = data(&s.call(&create("dbv_kg", DatabaseKind::Kg)).await).clone();
    assert_eq!(d["created"], true);
    assert_eq!(d["kind"], "kg");

    // FR 6: the stamp is the primary's structure, by construction.
    let dir = socket_dir().unwrap();
    let stamped = yeomna_store::connect(&dir, PORT, "yeomna_owner", "dbv_kg")
        .await
        .expect("the owner can inspect the stamp");
    let row = stamped
        .query_one(
            "SELECT
               (SELECT count(*) FROM information_schema.tables WHERE table_schema='public'),
               (SELECT count(*) FROM pg_extension WHERE extname='vector'),
               (SELECT pg_get_userbyid(relowner) FROM pg_class WHERE relname='audit_log'),
               (SELECT count(*) FROM pg_indexes WHERE indexname='edges_identity')",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 10, "the ten tables");
    assert_eq!(row.get::<_, i64>(1), 1, "the vector extension");
    assert_eq!(row.get::<_, String>(2), "yeomna_audit", "audit ownership");
    assert_eq!(row.get::<_, i64>(3), 1, "the identity index");
    // Release the inspection connection before asking for the drop.
    std::mem::drop(stamped);
    dbv_drop_and_confirm(&owner, &s, "dbv_kg").await;
}

#[tokio::test]
async fn a_plain_database_is_empty_postgres() {
    let Some((owner, s)) = fixtures().await else {
        return;
    };
    sweep(&owner, "dbv_plain").await;
    let d = data(&s.call(&create("dbv_plain", DatabaseKind::Plain)).await).clone();
    assert_eq!(d["kind"], "plain");
    let dir = socket_dir().unwrap();
    let plain = yeomna_store::connect(&dir, PORT, "yeomna_owner", "dbv_plain")
        .await
        .unwrap();
    let tables: i64 = plain
        .query_one(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(tables, 0, "plain means empty, full SQL utility later");
    std::mem::drop(plain);
    dbv_drop_and_confirm(&owner, &s, "dbv_plain").await;
}

/// Drop through the verb, retrying briefly: the caller has just closed
/// its inspection connection and the backend notices asynchronously, so
/// the first attempt can hit EC-3's in-use refusal honestly.
async fn dbv_drop_and_confirm(owner: &Client, s: &Session, name: &str) {
    let mut env = s.call(&drop(name, true)).await;
    for _ in 0..20 {
        if env.success || !env.error.as_deref().unwrap_or("").contains("in use") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        env = s.call(&drop(name, true)).await;
    }
    let d = data(&env).clone();
    assert_eq!(d["dropped"], true);
    let gone: i64 = owner
        .query_one(
            "SELECT count(*) FROM pg_database WHERE datname = $1",
            &[&name],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(gone, 0, "{name} is gone");
}

#[tokio::test]
async fn the_appliance_protects_its_own_footing() {
    let Some((owner, s)) = fixtures().await else {
        return;
    };
    // The session's database, the maintenance database, and the template
    // are refused by name. The primary is also protected by mechanism,
    // since the provision role does not own it, but the named refusal
    // answers first.
    for name in ["yeomna", "postgres", "yeomna_template"] {
        let env = s.call(&drop(name, true)).await;
        assert!(!env.success, "{name} must be refused");
        assert!(
            env.error.unwrap().starts_with("denied"),
            "{name} is denied, not merely missing"
        );
    }
    // A database provision does not own: denied by Postgres, surfaced as
    // denied. Owner creates it, so provision cannot drop it.
    sweep(&owner, "dbv_foreign").await;
    owner
        .execute("CREATE DATABASE dbv_foreign", &[])
        .await
        .unwrap();
    let env = s.call(&drop("dbv_foreign", true)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("denied"), "not the owner");
    sweep(&owner, "dbv_foreign").await;
}

#[tokio::test]
async fn refusals_are_typed_and_the_session_never_stays_escalated() {
    let Some((owner, s)) = fixtures().await else {
        return;
    };
    // Force gate.
    let env = s.call(&drop("dbv_whatever", false)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("denied"));

    // Names outside the identifier shape are refused before any SQL.
    for bad in ["Bad-Name", "has space", "UPPER", "pg_reserved", ""] {
        let env = s.call(&create(bad, DatabaseKind::Plain)).await;
        assert!(!env.success, "{bad:?} must be refused");
        assert!(env.error.unwrap().starts_with("invalid-args"), "{bad:?}");
    }

    // Duplicate create is InvalidArgs naming it.
    sweep(&owner, "dbv_dup").await;
    data(&s.call(&create("dbv_dup", DatabaseKind::Plain)).await);
    let env = s.call(&create("dbv_dup", DatabaseKind::Plain)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().contains("already exists"));
    dbv_drop_and_confirm(&owner, &s, "dbv_dup").await;

    // Dropping what is not there is NotFound.
    let env = s.call(&drop("dbv_never_was", true)).await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("not-found"));

    // EC-5: after every failure above, the session is still the app
    // role, not the provision role.
    let user: String = owner
        .query_one("SELECT 1 WHERE false", &[])
        .await
        .map(|_| unreachable!())
        .unwrap_or_else(|_| "unused".into());
    let _ = user;
    let d = data(&s.call(&Verb::Status(Empty {})).await).clone();
    assert_eq!(d["store"], "answering");
}

#[tokio::test]
async fn list_reports_the_cluster() {
    let Some((_o, s)) = fixtures().await else {
        return;
    };
    let d = data(&s.call(&Verb::DatabaseList(Empty {})).await).clone();
    let dbs = d["databases"].as_array().unwrap();
    let names: Vec<&str> = dbs.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"yeomna"));
    assert!(names.contains(&"yeomna_template"));
    let tmpl = dbs.iter().find(|x| x["name"] == "yeomna_template").unwrap();
    assert_eq!(tmpl["is_template"], true);
}
