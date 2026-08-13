//! The no-SQL-outside-the-line lint (spec 010, FR 7): SQL statement text
//! may live in `yeomna-store` and `yeomna-verbs` and nowhere else. The
//! verb layer is the only surface, and this test is the workspace-wide
//! enforcement of the charter's line, run on every `cargo test`.

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

fn scan() -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(crates_dir()).unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for f in std::fs::read_dir(&d).unwrap() {
                let p = f.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    found.extend(violations_in(&text, &p.display().to_string()));
                }
            }
        }
    }
    found
}

#[test]
fn no_sql_outside_the_line() {
    let found = scan();
    assert!(
        found.is_empty(),
        "SQL outside yeomna-store and yeomna-verbs:\n{}",
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
