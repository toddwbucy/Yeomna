//! No collapsed line continuation reaches a person (spec 022).
//!
//! A message written across two source lines is easy to get wrong. The
//! intended shape is a `\` at the end of the line, which makes Rust strip
//! the newline and the next line's indentation. Leave the `\` out, or lose
//! it to a tool that rewrites the file, and the indentation stays in the
//! string: a caller reads "Phase 3.              The embedder is here"
//! with fourteen spaces in the middle of a sentence.
//!
//! This has shipped once already, in a pipeline refusal on main since PR
//! #30, and it happened again while building the embedder. Twice is a
//! class, and a class gets a mechanical check rather than a resolution to
//! be more careful.
//!
//! SQL is exempt. Statement text is indented inside its literal on
//! purpose, and it is not prose a person reads as a sentence.

use std::path::{Path, PathBuf};

/// The run of spaces that means a continuation collapsed. Two spaces
/// happen legitimately (a column pad, a sentence break in older style), so
/// the threshold is three, which no intentional message has.
const RUN: &str = "   ";

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/yeomna-verbs has a parent")
        .to_path_buf()
}

/// Does this literal look like SQL rather than a sentence.
fn is_sql(body: &str) -> bool {
    const KEYWORDS: [&str; 12] = [
        "SELECT",
        "INSERT",
        "DELETE FROM",
        "UPDATE ",
        "WITH RECURSIVE",
        " FROM ",
        " JOIN ",
        "VALUES",
        "ON CONFLICT",
        "GROUP BY",
        "ORDER BY",
        "CREATE ",
    ];
    let upper = body.to_ascii_uppercase();
    KEYWORDS.iter().any(|k| upper.contains(k))
}

/// Every string literal on one line, roughly. Good enough for a lint: it
/// wants no false negatives on the shape being hunted, and its own
/// negative test proves it still detects.
fn literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut body = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    // Keep the escape as written, so `\n` in a source
                    // fixture stays two characters and does not read as a
                    // line break followed by indentation.
                    body.push('\\');
                    if let Some(n) = chars.next() {
                        body.push(n);
                    }
                }
                '"' => break,
                c => body.push(c),
            }
        }
        out.push(body);
    }
    out
}

/// A run of spaces inside a sentence, which is what a collapsed
/// continuation leaves.
///
/// Two things that look the same and are not. A run after an escaped
/// newline is a source-code fixture, where `"fn a() {\\n    1\\n}"` is
/// indented Rust and the indentation is the point. And a run at either end
/// of the literal is padding rather than a break in a sentence. Only a run
/// between two non-space characters, not preceded by an escape, is the
/// thing being hunted.
fn has_collapsed_run(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0;
    while let Some(rel) = body[i..].find(RUN) {
        let at = i + rel;
        let end = at + body[at..].len() - body[at..].trim_start_matches(' ').len();
        let before = &body[..at];
        let after_is_text = body[end..].chars().next().is_some_and(|c| c != ' ');
        let before_is_text = before.chars().next_back().is_some_and(|c| c != ' ');
        // `\n`, `\r`, `\t` immediately before the run means a fixture.
        let after_escape = before.len() >= 2
            && bytes[before.len() - 2] == b'\\'
            && matches!(bytes[before.len() - 1], b'n' | b'r' | b't');
        if before_is_text && after_is_text && !after_escape {
            return true;
        }
        i = end.max(at + 1);
    }
    false
}

#[test]
fn no_message_carries_a_collapsed_continuation() {
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    let mut stack = vec![crates_dir()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // This file's own examples are the shape being hunted.
                if path.file_name().is_some_and(|n| n == "message_lint.rs") {
                    continue;
                }
                scanned += 1;
                let text = std::fs::read_to_string(&path).expect("readable");
                for (n, line) in text.lines().enumerate() {
                    for body in literals(line) {
                        if has_collapsed_run(&body) && !is_sql(&body) {
                            offenders.push(format!(
                                "{}:{}: {}",
                                path.display(),
                                n + 1,
                                body.chars().take(90).collect::<String>()
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        scanned > 20,
        "scanned {scanned} files, so a pass means little"
    );
    assert!(
        offenders.is_empty(),
        "a message written across two lines lost its `\\` and kept the \
         indentation. Add the backslash, or join the literal:\n{}",
        offenders.join("\n")
    );
}

/// The lint's own negative test: a planted collapse is caught, so a clean
/// result means detection worked rather than detection broke.
#[test]
fn the_lint_catches_a_planted_collapse() {
    assert!(has_collapsed_run(
        "hybrid ranking needs the fusion.              The embedder is here"
    ));
    assert!(has_collapsed_run("a b   c"));
    // Two spaces are not a collapse.
    assert!(!has_collapsed_run("one sentence.  Another one."));
    // Leading and trailing runs are padding, not a collapse.
    assert!(!has_collapsed_run("   indented"));
    assert!(!has_collapsed_run("trailing   "));
    // A source fixture's escaped newline is two characters, not a break.
    assert!(!has_collapsed_run("fn a() {\\n    1\\n}"));

    // And the extractor: a literal is read out of a line the way the scan
    // reads it, escapes intact.
    let found = literals(r#"let a = "one   two"; let b = "fine";"#);
    assert_eq!(found, vec!["one   two".to_string(), "fine".to_string()]);
    assert!(is_sql("SELECT id FROM nodes   WHERE x = 1"));
    assert!(!is_sql("the embedder does not serve that task"));
}
