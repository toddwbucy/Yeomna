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
/// happen legitimately (a sentence break in older style), so the threshold
/// is three.
const RUN: &str = "   ";

/// The shortest literal this lint judges.
///
/// A collapsed continuation always spans two source lines, and two lines of
/// Rust at any normal indentation is far longer than this. What is shorter
/// is padding: a rendered table row like `"key   depth"` is column
/// alignment, and a test that pins one is asserting the alignment. Spec
/// 021's renderer tests are full of them, and they are correct.
const MIN_PROSE: usize = 60;

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

/// Every string literal in a whole file, with the line it opened on.
///
/// Whole-file rather than line-by-line, which is the difference between a
/// lint that works and one that does not. A literal whose continuation
/// backslash is missing **spans two source lines**, so a line-by-line scan
/// never sees the text before the break and the indentation after it in the
/// same string, and passes the exact malformed literal it exists to reject.
/// A raw string (`r"..."`, `r#"..."#`) is skipped: escapes mean nothing
/// inside one and none of them here is a message.
fn literals(text: &str) -> Vec<(usize, String)> {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        // A line comment: nothing in it is a literal, and a `"` inside one
        // would otherwise open a literal that swallows the rest of the file.
        if c == '/' && bytes.get(i + 1) == Some(&'/') {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // A raw string, in either of its forms. Skipped by finding its
        // matching terminator so its contents cannot be mistaken for a
        // normal literal's.
        if c == 'r' && matches!(bytes.get(i + 1), Some('"') | Some('#')) {
            let mut hashes = 0usize;
            let mut j = i + 1;
            while bytes.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) == Some(&'"') {
                let close: String = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                let rest: String = bytes[j + 1..].iter().collect();
                let end = rest
                    .find(&close)
                    .map(|at| j + 1 + rest[..at].chars().count());
                let stop = end.unwrap_or(bytes.len());
                line += bytes[i..stop].iter().filter(|c| **c == '\n').count();
                i = stop + close.chars().count();
                continue;
            }
        }
        // A char literal, so `'"'` does not open a string.
        if c == '\'' && bytes.get(i + 1) == Some(&'"') && bytes.get(i + 2) == Some(&'\'') {
            i += 3;
            continue;
        }
        if c != '"' {
            i += 1;
            continue;
        }
        let opened = line;
        let mut body = String::new();
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                '\\' => {
                    // Keep the escape as written, so `\n` in a source
                    // fixture stays two characters and does not read as a
                    // line break followed by indentation. A backslash before
                    // a real newline is the continuation this lint is about,
                    // and Rust removes the newline and the indentation, so
                    // the literal is recorded the way Rust sees it.
                    if bytes.get(i + 1) == Some(&'\n') {
                        line += 1;
                        i += 2;
                        while matches!(bytes.get(i), Some(' ') | Some('\t')) {
                            i += 1;
                        }
                        continue;
                    }
                    body.push('\\');
                    if let Some(n) = bytes.get(i + 1) {
                        body.push(*n);
                    }
                    i += 2;
                }
                '"' => {
                    i += 1;
                    break;
                }
                c => {
                    if c == '\n' {
                        line += 1;
                    }
                    body.push(c);
                    i += 1;
                }
            }
        }
        out.push((opened, body));
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
    if body.len() < MIN_PROSE {
        return false;
    }
    // The split form: a real newline inside the literal followed by
    // indentation. That is exactly what a missing continuation backslash
    // leaves, and it reaches a reader as a line break and a wall of spaces
    // in the middle of a sentence.
    for (i, _) in body.match_indices('\n') {
        let after = &body[i + 1..];
        let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
        if spaces >= RUN.len() && after[spaces..].starts_with(|c: char| c != '\n') {
            return true;
        }
    }
    // The joined form: the same mistake after something has put the literal
    // back on one line, which is how it reached this repository twice.
    let bytes = body.as_bytes();
    let mut i = 0;
    while let Some(rel) = body[i..].find(RUN) {
        let at = i + rel;
        let end = at + body[at..].len() - body[at..].trim_start_matches(' ').len();
        let before = &body[..at];
        let after_is_text = body[end..].chars().next().is_some_and(|c| c != ' ');
        let before_is_text = before.chars().next_back().is_some_and(|c| c != ' ');
        // `\n`, `\r`, `\t` immediately before the run means a fixture,
        // where the indentation is the point.
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
                for (line, body) in literals(&text) {
                    if has_collapsed_run(&body) && !is_sql(&body) {
                        offenders.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            line,
                            body.chars()
                                .take(90)
                                .collect::<String>()
                                .replace('\n', "<NL>")
                        ));
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
    // Two spaces are not a collapse.
    assert!(!has_collapsed_run(
        "one sentence.  Another one, and enough text to be judged as prose at all."
    ));
    // Leading and trailing runs are padding, not a collapse.
    assert!(!has_collapsed_run(
        "   indented, and long enough that the length rule is not what excused it."
    ));
    assert!(!has_collapsed_run(
        "trailing, and long enough that the length rule is not what excused it.   "
    ));
    // A rendered table row is column alignment, and a test pinning one is
    // asserting that alignment. Short, which is how it is told apart from a
    // message that lost a line break.
    assert!(!has_collapsed_run("key   depth"));
    assert!(!has_collapsed_run("----  -----"));
    assert!(!has_collapsed_run("bbbb  12"));
    assert!(!has_collapsed_run("a b   c"));
    // A source fixture's escaped newline is two characters, not a break.
    assert!(!has_collapsed_run("fn a() {\\n    1\\n}"));

    // The split form, which is the one a line-by-line scan cannot see and
    // the one this lint exists for. A literal whose continuation backslash
    // is missing spans two source lines, so the text before the break and
    // the indentation after it are only in the same string if the whole file
    // was parsed.
    assert!(has_collapsed_run(
        "hybrid ranking needs the fusion.\n             The embedder is here"
    ));
    // And a literal that was written across lines *with* its backslash
    // reaches the lint already joined, because the extractor removes the
    // newline and the indentation the way Rust does.
    let joined = literals(
        "let m = \"a message long enough to have been written across two \\\n                      source lines, which is what makes it prose\";",
    );
    assert_eq!(joined.len(), 1, "{joined:?}");
    assert_eq!(
        joined[0].1,
        "a message long enough to have been written across two source lines, \
         which is what makes it prose"
    );
    assert!(!has_collapsed_run(&joined[0].1));

    // Without the backslash the same shape survives into the literal, and
    // the lint catches it.
    let split = literals(
        "let m = \"a message long enough to have been written across two\n                      source lines, which is what makes it prose\";",
    );
    assert_eq!(split.len(), 1, "{split:?}");
    assert!(
        has_collapsed_run(&split[0].1),
        "the missing backslash is the whole point: {:?}",
        split[0].1
    );

    // The extractor, on the shapes that would otherwise confuse it.
    let found = literals(r##"let a = "one   two"; let b = "fine";"##);
    assert_eq!(
        found.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>(),
        vec!["one   two".to_string(), "fine".to_string()]
    );
    // A `"` inside a line comment does not open a literal.
    let commented = literals("// a quote \" here\nlet a = \"real\";");
    assert_eq!(
        commented.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>(),
        vec!["real".to_string()]
    );
    // A char literal holding a quote does not either.
    let charred = literals("if c == '\"' { } let a = \"real\";");
    assert_eq!(
        charred.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>(),
        vec!["real".to_string()]
    );
    // The line number is where the literal opened.
    let numbered = literals("fn a() {}\nfn b() {}\nlet m = \"here\";");
    assert_eq!(numbered[0].0, 3);

    assert!(is_sql("SELECT id FROM nodes   WHERE x = 1"));
    assert!(!is_sql("the embedder does not serve that task"));
}
