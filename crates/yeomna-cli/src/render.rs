//! Envelope to text, for the person typing rather than the program
//! calling (spec 021).
//!
//! Presentation only. Nothing here decides what a verb means, and the
//! same envelope `--json` prints verbatim is what this renders, so the
//! two views can never disagree about the answer.
//!
//! Headers and rows go to the same stream. The captured CLI printed
//! headers to stderr and rows to stdout, so redirecting stdout lost the
//! header, and that is one of the five things PR #19's review said not
//! to repeat.

use serde_json::Value;
use yeomna_verbs::Envelope;

/// The whole envelope as a person reads it.
pub fn envelope(e: &Envelope) -> String {
    let mut out = String::new();
    match (&e.data, &e.error) {
        (Some(data), _) => {
            out.push_str(&value(data, 0));
        }
        (None, Some(err)) => {
            // Through the sanitizer like anything else: an error can carry
            // a key or a path the caller supplied, and a caller is not
            // always a person typing.
            out.push_str(&printable(err));
            out.push('\n');
        }
        (None, None) => {
            out.push_str("(no data)\n");
        }
    }
    out
}

/// A JSON value as indented text, with arrays of like-shaped objects
/// rendered as a table because that is the shape a person scans.
fn value(v: &Value, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(map) => {
            let mut out = String::new();
            let width = map
                .keys()
                .filter(|k| !map[k.as_str()].is_array() && !map[k.as_str()].is_object())
                .map(String::len)
                .max()
                .unwrap_or(0);
            // Scalars first, so the shape of a thing reads before its
            // contents.
            for (k, val) in map {
                if val.is_array() || val.is_object() {
                    continue;
                }
                out.push_str(&format!("{pad}{k:<width$}  {}\n", scalar(val)));
            }
            for (k, val) in map {
                match val {
                    Value::Array(items) if items.is_empty() => {
                        out.push_str(&format!("{pad}{k}: none\n"));
                    }
                    Value::Array(items) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        out.push_str(&array(items, depth + 1));
                    }
                    Value::Object(_) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        out.push_str(&value(val, depth + 1));
                    }
                    _ => {}
                }
            }
            out
        }
        Value::Array(items) => array(items, depth),
        other => format!("{pad}{}\n", scalar(other)),
    }
}

/// An array: a table when its elements are objects sharing a shape, a
/// list otherwise.
fn array(items: &[Value], depth: usize) -> String {
    let pad = "  ".repeat(depth);
    let columns = shared_columns(items);
    if let Some(cols) = columns {
        let mut widths: Vec<usize> = cols.iter().map(String::len).collect();
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|it| {
                cols.iter()
                    .map(|c| cell(it.get(c).unwrap_or(&Value::Null)))
                    .collect()
            })
            .collect();
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let mut out = String::new();
        // The header rides the same stream as the rows.
        let header: Vec<String> = cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect();
        out.push_str(&format!("{pad}{}\n", header.join("  ").trim_end()));
        out.push_str(&format!(
            "{pad}{}\n",
            widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ")
        ));
        for row in rows {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| format!("{cell:<w$}", w = widths[i]))
                .collect();
            out.push_str(&format!("{pad}{}\n", cells.join("  ").trim_end()));
        }
        out
    } else {
        items
            .iter()
            .map(|it| match it {
                Value::Object(_) | Value::Array(_) => value(it, depth),
                other => format!("{pad}{}\n", scalar(other)),
            })
            .collect()
    }
}

/// The columns an array of objects shares, in first-seen order, or
/// `None` when the elements are not uniformly shaped objects with
/// scalar fields.
fn shared_columns(items: &[Value]) -> Option<Vec<String>> {
    let first = items.first()?.as_object()?;
    if first.is_empty() {
        return None;
    }
    let cols: Vec<String> = first.keys().cloned().collect();
    for it in items {
        let obj = it.as_object()?;
        if obj.len() != cols.len() || cols.iter().any(|c| !obj.contains_key(c)) {
            return None;
        }
        // Every row is checked, not only the first. A nested value in a
        // later row would otherwise be serialized into a cell as raw JSON,
        // which is a table that lies about its shape rather than a list
        // that admits it.
        if obj.values().any(|v| v.is_array() || v.is_object()) {
            return None;
        }
    }
    Some(cols)
}

/// The widest a table cell gets before it is cut.
///
/// `query` returns whole chunks of source in a `text` column, and one of
/// those in a cell makes a table that no terminal can lay out and no
/// person can read. This view exists for the person typing, and `--json`
/// prints the value in full for anything that needs it, so cutting here
/// loses nothing that was not already available.
const MAX_CELL: usize = 72;

/// Make a string safe to write to a terminal.
///
/// Everything here came out of the store, and what is in the store came
/// out of an ingested corpus, which means it is not this program's text.
/// A chunk of source containing an escape sequence would otherwise reach
/// the terminal as a command: clear the screen, move the cursor, change
/// the title, or worse on terminals with more ambitious escapes. Rendering
/// is the boundary where corpus text becomes terminal output, so it is
/// where the escaping belongs.
///
/// Tab, newline, and carriage return become one space, because they are
/// ordinary in source text and a table row cannot contain them. Every
/// other control character, DEL, and the C1 range become a visible
/// `\xNN`, so the value is neither obeyed nor silently dropped. `--json`
/// prints the bytes as they are for anything that needs them.
fn printable(text: &str) -> String {
    if !text
        .chars()
        .any(|c| c.is_control() || ('\u{80}'..='\u{9f}').contains(&c))
    {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' | '\n' | '\r' => out.push(' '),
            c if c.is_control() || ('\u{80}'..='\u{9f}').contains(&c) => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// A scalar as a cell: strings without their quotes, null as a dash,
/// everything else as JSON writes it.
///
/// Strings go through [`printable`], which is the single funnel every
/// value that reaches stdout passes through.
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => printable(s),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

/// A cell, on one line and inside the width.
///
/// Newlines become spaces before the cut, because a cell containing one
/// breaks the row it is in and the alignment of every row after it.
fn cell(v: &Value) -> String {
    // `scalar` has already turned every control character into a space or
    // a visible escape, so a run of whitespace is all that is left to
    // collapse and no cell can carry a line break into a row.
    let raw = scalar(v);
    let flat = if raw.contains("  ") || raw.starts_with(' ') || raw.ends_with(' ') {
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        raw
    };
    if flat.chars().count() <= MAX_CELL {
        return flat;
    }
    let kept: String = flat.chars().take(MAX_CELL - 3).collect();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_align_and_come_before_their_collections() {
        let out = value(&json!({"graph": "g", "nodes": 3, "items": [1, 2]}), 0);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("graph"), "{out}");
        assert!(lines[1].starts_with("nodes"), "{out}");
        assert!(
            lines[2].starts_with("items:"),
            "the collection is last: {out}"
        );
    }

    /// The header is on the same stream and the same string as the rows,
    /// which is the point (PR #19's second do-not-repeat).
    #[test]
    fn like_shaped_objects_render_as_one_table() {
        let out = array(
            &[
                json!({"key": "a", "depth": 0}),
                json!({"key": "bbbb", "depth": 12}),
            ],
            0,
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "key   depth");
        assert_eq!(lines[1], "----  -----");
        assert_eq!(lines[2], "a     0");
        assert_eq!(lines[3], "bbbb  12");
    }

    #[test]
    fn unlike_shapes_are_a_list_rather_than_a_wrong_table() {
        let out = array(&[json!({"a": 1}), json!({"b": 2})], 0);
        assert!(
            !out.contains("----"),
            "no table for mismatched shapes: {out}"
        );
        let out = array(&[json!({"a": {"nested": 1}})], 0);
        assert!(!out.contains("----"), "no table for nested values: {out}");
        // A nested value in a later row disqualifies the table too, which
        // checking only the first element would have missed.
        for later in [json!({"a": {"nested": 2}}), json!({"a": [1, 2]})] {
            let out = array(&[json!({"a": 1}), later.clone()], 0);
            assert!(
                !out.contains("----"),
                "no table when a later row nests: {out}"
            );
            assert!(
                !out.contains("nested") || !out.contains("  a  "),
                "and the nested value is not serialized into a cell: {out}"
            );
        }
    }

    /// A cell wide enough to break the table is cut, and a cell with a
    /// newline in it is flattened first, because either one destroys the
    /// alignment of every row after it. `--json` still prints the value in
    /// full, so nothing is lost that was not already reachable.
    #[test]
    fn a_wide_or_multiline_cell_does_not_break_the_table() {
        let long = "x".repeat(500);
        let out = array(
            &[
                json!({"k": "a", "v": long}),
                json!({"k": "b", "v": "short"}),
            ],
            0,
        );
        for line in out.lines() {
            assert!(
                line.chars().count() < 120,
                "no line is wider than a terminal: {} chars",
                line.chars().count()
            );
        }
        assert!(out.contains("..."), "the cut is marked: {out}");

        let out = array(&[json!({"k": "a", "v": "one\ntwo\nthree"})], 0);
        assert_eq!(
            out.lines().count(),
            3,
            "header, rule, and one row, not a row per newline: {out}"
        );
        assert!(out.contains("one two three"), "{out}");
    }

    /// Corpus text is not this program's text, and a terminal obeys
    /// escape sequences. Nothing that came out of the store reaches stdout
    /// able to command the terminal it is printed to.
    #[test]
    fn control_characters_from_the_corpus_cannot_reach_the_terminal() {
        let hostile = "before\u{1b}[2Jafter\u{7}\u{7f}\u{9b}end";
        // Through a table cell.
        let out = array(&[json!({"text": hostile})], 0);
        assert!(!out.contains('\u{1b}'), "no ESC survives: {out:?}");
        assert!(!out.contains('\u{7}'), "no BEL survives: {out:?}");
        assert!(!out.contains('\u{7f}'), "no DEL survives: {out:?}");
        assert!(!out.contains('\u{9b}'), "no C1 CSI survives: {out:?}");
        assert!(out.contains("\\x1b"), "it is visible instead: {out:?}");
        assert!(out.contains("before") && out.contains("end"), "{out:?}");

        // Through the scalar path, which is not a table at all.
        let out = value(&json!({"k": hostile}), 0);
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert!(out.contains("\\x1b"), "{out:?}");

        // Through an error, which can carry a key the caller supplied.
        let e = yeomna_verbs::error_envelope(
            "get",
            &yeomna_verbs::VerbError::NotFound(format!("no document named {hostile:?}")),
        );
        let out = envelope(&e);
        assert!(!out.contains('\u{1b}'), "{out:?}");

        // And a value with no control characters is untouched, so the
        // sanitizer is not quietly rewriting ordinary text.
        assert_eq!(
            printable("plain \u{4e16}\u{754c} text"),
            "plain \u{4e16}\u{754c} text"
        );
    }

    #[test]
    fn an_empty_array_says_none_rather_than_showing_nothing() {
        let out = value(&json!({"missing": []}), 0);
        assert_eq!(out, "missing: none\n");
    }

    #[test]
    fn a_failure_renders_its_error_and_nothing_else() {
        let e = yeomna_verbs::error_envelope(
            "get",
            &yeomna_verbs::VerbError::NotFound("no document named \"x\"".into()),
        );
        let out = envelope(&e);
        assert!(out.starts_with("not-found: no document named"), "{out}");
    }
}
