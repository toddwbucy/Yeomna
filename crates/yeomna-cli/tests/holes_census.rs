//! The holes census: every captured command runs, emits the hole envelope,
//! and exits nonzero. This list is the ledger in machine-checked form.

use std::process::Command;

const CENSUS: &[(&[&str], &str)] = &[
    (&["status"], "status"),
    (&["orient"], "orient"),
    (&["extract", "/tmp/x.pdf"], "extract"),
    (&["ingest", "/tmp/x.pdf"], "ingest"),
    (&["daemon"], "daemon"),
    (&["db", "query"], "db query"),
    (&["db", "list"], "db list"),
    (&["db", "stats"], "db stats"),
    (&["db", "recent"], "db recent"),
    (&["db", "health"], "db health"),
    (&["db", "check", "x"], "db check"),
    (&["db", "purge", "x"], "db purge"),
    (&["db", "create", "x"], "db create"),
    (&["db", "delete", "x", "1"], "db delete"),
    (&["db", "collections"], "db collections"),
    (&["db", "databases"], "db databases"),
    (&["db", "create-database", "x"], "db create-database"),
    (&["db", "truncate", "x"], "db truncate"),
    (&["db", "drop-collection", "x"], "db drop-collection"),
    (&["db", "count", "x"], "db count"),
    (&["db", "get", "x", "1"], "db get"),
    (&["db", "insert", "x"], "db insert"),
    (&["db", "update", "x", "1"], "db update"),
    (&["db", "export", "x"], "db export"),
    (&["db", "create-index"], "db create-index"),
    (&["db", "index-status"], "db index-status"),
    (&["db", "graph", "create", "x"], "db graph create"),
    (&["db", "graph", "list"], "db graph list"),
    (&["db", "graph", "drop", "x"], "db graph drop"),
    (&["db", "graph", "traverse", "x"], "db graph traverse"),
    (
        &["db", "graph", "shortest-path", "x", "x"],
        "db graph shortest-path",
    ),
    (&["db", "graph", "neighbors", "x"], "db graph neighbors"),
    (&["db", "graph", "materialize"], "db graph materialize"),
    (&["db", "schema", "init", "--seed", "x"], "db schema init"),
    (&["db", "schema", "list"], "db schema list"),
    (&["db", "schema", "show", "x"], "db schema show"),
    (&["db", "schema", "version"], "db schema version"),
    (&["embed", "text", "x"], "embed text"),
    (&["embed", "service", "status"], "embed service status"),
    (&["embed", "service", "start"], "embed service start"),
    (&["embed", "service", "stop"], "embed service stop"),
    (&["embed", "gpu", "status"], "embed gpu status"),
    (&["embed", "gpu", "list"], "embed gpu list"),
    (&["codebase", "ingest", "x"], "codebase ingest"),
    (&["codebase", "update", "x"], "codebase update"),
    (&["codebase", "stats"], "codebase stats"),
    (&["codebase", "validate"], "codebase validate"),
    (&["codebase", "prune-orphans"], "codebase prune-orphans"),
    (&["codebase", "drift", "x"], "codebase drift"),
    (&["codebase", "retire"], "codebase retire"),
    (&["graph-embed", "embed", "x"], "graph-embed embed"),
    (&["graph-embed", "neighbors", "x"], "graph-embed neighbors"),
    (&["graph-embed", "update"], "graph-embed update"),
    (&["schema", "apply", "x"], "schema apply"),
    (&["tools", "status"], "tools status"),
    (&["tools", "install", "x"], "tools install"),
];

#[test]
fn every_command_is_a_self_reporting_hole() {
    for (argv, name) in CENSUS {
        let out = Command::new(env!("CARGO_BIN_EXE_yeomna"))
            .args(*argv)
            .output()
            .expect("binary runs");
        assert!(!out.status.success(), "{name}: a hole must exit nonzero");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("{name}: envelope not JSON ({e}): {stdout}"));
        assert_eq!(v["hole"], true, "{name}");
        assert_eq!(v["success"], false, "{name}");
        assert!(
            v["error"].as_str().unwrap_or("").starts_with("hole: "),
            "{name}: {}",
            v["error"]
        );
    }
    eprintln!("census: {} holes verified", CENSUS.len());
}
