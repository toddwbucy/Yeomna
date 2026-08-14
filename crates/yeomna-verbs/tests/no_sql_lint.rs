//! The no-SQL-outside-the-line lint (spec 010, FR 7): SQL statement text
//! may live in `yeomna-store` and `yeomna-verbs` and nowhere else. The
//! verb layer is the only surface, and this test is the workspace-wide
//! enforcement of the charter's line, run on every `cargo test`.
//!
//! Scope: every Rust file in every other crate, not only `src/`. A
//! traversal hand-written into a test or a benchmark leaks the same
//! knowledge as one in a module, so `tests/`, `benches/`, `examples/`,
//! and `build.rs` are all in scope.
//!
//! `yeomna-verbs` stays allowed alongside `yeomna-store` because the
//! verb-layer PRD makes it the SQL author above the sink from Phase 2
//! on. The line this lint draws is around the two crates that own the
//! store, not around the store crate alone.

use std::path::{Path, PathBuf};

/// The crates allowed to carry SQL.
const ALLOWED: [&str; 2] = ["yeomna-store", "yeomna-verbs"];

/// Statement-shaped fragments that mark SQL in source text. Conservative
/// on purpose: prose about a SELECT does not match, statement text does.
const PATTERNS: [&str; 6] = [
    "SELECT ",
    "INSERT INTO ",
    "DELETE FROM ",
    "CREATE TABLE ",
    "UPDATE nodes",
    "WITH RECURSIVE",
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn violations_in(src: &str, origin: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        // Only statement text in string literals matters. A line with a
        // quote and a pattern is close enough to that without a parser.
        if line.contains('"') && PATTERNS.iter().any(|p| line.contains(p)) {
            out.push(format!("{origin}:{}: {}", n + 1, line.trim()));
        }
    }
    out
}

/// Violations found, and how many Rust files were read to find them. The
/// count is asserted below: a walk that silently visits nothing would
/// otherwise report a clean workspace forever.
fn scan() -> (Vec<String>, usize) {
    let mut found = Vec::new();
    let mut visited = 0;
    for entry in std::fs::read_dir(crates_dir()).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for f in std::fs::read_dir(&d).unwrap() {
                let p = f.unwrap().path();
                if p.is_dir() {
                    // Build output is not source.
                    if p.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    visited += 1;
                    let text = std::fs::read_to_string(&p).unwrap();
                    found.extend(violations_in(&text, &p.display().to_string()));
                }
            }
        }
    }
    (found, visited)
}

#[test]
fn no_sql_outside_the_line() {
    let (found, visited) = scan();
    assert!(
        visited > 0,
        "the lint visited no Rust files, so its clean result means nothing"
    );
    assert!(
        found.is_empty(),
        "SQL outside yeomna-store and yeomna-verbs ({visited} files scanned):\n{}",
        found.join("\n")
    );
}

/// The lint's negative test (spec 010 success criterion 4): a planted
/// violation is caught, so a green lint means detection works, not that
/// detection is broken.
#[test]
fn the_lint_catches_a_planted_violation() {
    let planted = r#"let q = "SELECT id FROM nodes WHERE graph_id = $1";"#;
    let hits = violations_in(planted, "planted.rs");
    assert_eq!(hits.len(), 1, "planted SQL must be caught");
}
