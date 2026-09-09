//! The scoped `sql` verb (spec 014 FR8, EC-7, EC-8): full utility on
//! plain databases, structural refusal on the kg pattern, footing
//! refused by name.
//!
//! Needs the provisioning that spec 013 documented: the provision role
//! for lifecycle and execution, and the template for the kg stamp test.

use serde_json::json;
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

/// A session with an endpoint, which is what `sql` needs, plus the owner
/// for inspection and cleanup. Skips name their causes per house rule.
async fn fixtures(actor: &str) -> Option<(String, Client, Session)> {
    let dir = socket_dir()?;
    let Ok(owner) = yeomna_store::connect(&dir, PORT, "yeomna_owner", "yeomna").await else {
        eprintln!("SKIP: cannot connect as yeomna_owner");
        return None;
    };
    for role in ["yeomna_app", "yeomna_provision"] {
        let present: bool = owner
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)",
                &[&role],
            )
            .await
            .unwrap()
            .get(0);
        if !present {
            eprintln!("SKIP: role {role} is not provisioned on this cluster");
            return None;
        }
    }
    yeomna_store::apply_schema(&owner).await.expect("schema");
    let app = yeomna_store::connect(&dir, PORT, "yeomna_app", "yeomna")
        .await
        .expect("yeomna_app exists, so connecting as it must succeed");
    let session = Session::new(app, actor).with_endpoint(dir.clone(), PORT);
    Some((dir, owner, session))
}

/// Drop a scratch database if an earlier failed run left it behind.
async fn drop_db(s: &Session, name: &str) {
    s.call(&Verb::DatabaseDrop(DropRequest {
        name: name.into(),
        force: true,
    }))
    .await;
}

#[tokio::test]
async fn sql_has_full_utility_on_a_plain_database() {
    let Some((_dir, _owner, s)) = fixtures("sqlv-plain").await else {
        return;
    };
    drop_db(&s, "sqlv_plain").await;
    let env = s
        .call(&Verb::DatabaseCreate(DatabaseCreateRequest {
            name: "sqlv_plain".into(),
            kind: DatabaseKind::Plain,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);

    let env = s
        .call(&Verb::Sql(SqlRequest {
            database: "sqlv_plain".into(),
            statement: "CREATE TABLE t (a int); INSERT INTO t VALUES (1), (2); \
                        SELECT a FROM t ORDER BY a"
                .into(),
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let d = env.data.unwrap();
    assert_eq!(d["role"], "yeomna_provision");
    let results = d["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "one result per statement");
    assert_eq!(results[1]["rows_affected"], 2);
    assert_eq!(results[2]["rows"], json!([["1"], ["2"]]));
    assert_eq!(results[2]["columns"], json!(["a"]));

    drop_db(&s, "sqlv_plain").await;
}

#[tokio::test]
async fn sql_refuses_the_kg_pattern_wherever_it_came_from() {
    let Some((dir, _owner, s)) = fixtures("sqlv-kg").await else {
        return;
    };
    let template: bool = _owner
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'yeomna_template')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    if !template {
        eprintln!("SKIP: yeomna_template is not provisioned on this cluster");
        return;
    }
    // A stamped kg database refuses.
    drop_db(&s, "sqlv_kg").await;
    let env = s
        .call(&Verb::DatabaseCreate(DatabaseCreateRequest {
            name: "sqlv_kg".into(),
            kind: DatabaseKind::Kg,
        }))
        .await;
    assert!(env.success, "{:?}", env.error);
    let env = s
        .call(&Verb::Sql(SqlRequest {
            database: "sqlv_kg".into(),
            statement: "SELECT 1".into(),
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("denied") && msg.contains("kg pattern"),
        "{msg}"
    );
    drop_db(&s, "sqlv_kg").await;

    // EC-7: a plain database hand-stamped with one signature table
    // refuses the same way, which is why R17a chose structure over a
    // registry.
    drop_db(&s, "sqlv_hand").await;
    s.call(&Verb::DatabaseCreate(DatabaseCreateRequest {
        name: "sqlv_hand".into(),
        kind: DatabaseKind::Plain,
    }))
    .await;
    let hand = yeomna_store::connect(&dir, PORT, "yeomna_owner", "sqlv_hand")
        .await
        .expect("the owner reaches the new plain database");
    hand.batch_execute("CREATE TABLE nodes (id int)")
        .await
        .unwrap();
    let env = s
        .call(&Verb::Sql(SqlRequest {
            database: "sqlv_hand".into(),
            statement: "SELECT 1".into(),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().contains("kg pattern"), "EC-7");
    std::mem::drop(hand);
    // The owner's table blocks a provision-role drop of contents but not
    // the database drop itself, which the provision role owns.
    drop_db(&s, "sqlv_hand").await;
}

#[tokio::test]
async fn sql_refuses_the_footing_by_name() {
    let Some((_dir, _owner, s)) = fixtures("sqlv-footing").await else {
        return;
    };
    for name in ["yeomna", "postgres", "yeomna_template"] {
        let env = s
            .call(&Verb::Sql(SqlRequest {
                database: name.to_string(),
                statement: "SELECT 1".into(),
            }))
            .await;
        assert!(!env.success, "{name} must refuse");
        assert!(env.error.unwrap().starts_with("denied"), "{name}");
    }
    let env = s
        .call(&Verb::Sql(SqlRequest {
            database: "sqlv_absent".into(),
            statement: "SELECT 1".into(),
        }))
        .await;
    assert!(!env.success);
    assert!(env.error.unwrap().starts_with("not-found"));
}

/// EC-8: a failing statement surfaces the taxonomy kind with the
/// SQLSTATE in the detail, and the audit row records the kind.
#[tokio::test]
async fn a_failing_statement_carries_its_sqlstate() {
    let Some((_dir, owner, s)) = fixtures("sqlv-fail").await else {
        return;
    };
    drop_db(&s, "sqlv_fail").await;
    s.call(&Verb::DatabaseCreate(DatabaseCreateRequest {
        name: "sqlv_fail".into(),
        kind: DatabaseKind::Plain,
    }))
    .await;
    owner
        .execute("DELETE FROM audit_log WHERE actor = 'sqlv-fail'", &[])
        .await
        .unwrap();
    let env = s
        .call(&Verb::Sql(SqlRequest {
            database: "sqlv_fail".into(),
            statement: "SELECT nothing FROM nowhere".into(),
        }))
        .await;
    assert!(!env.success);
    let msg = env.error.unwrap();
    assert!(
        msg.starts_with("invalid-args") && msg.contains("42P01"),
        "{msg}"
    );
    let outcome: Option<String> = owner
        .query_one(
            "SELECT outcome FROM audit_log WHERE actor = 'sqlv-fail' AND verb = 'sql'
             ORDER BY id DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(outcome.as_deref(), Some("failed: invalid-args"));
    drop_db(&s, "sqlv_fail").await;
}
