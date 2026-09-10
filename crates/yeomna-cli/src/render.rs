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
            out.push_str(err);
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
                    .map(|c| scalar(it.get(c).unwrap_or(&Value::Null)))
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
    if first.is_empty() || first.values().any(|v| v.is_array() || v.is_object()) {
        return None;
    }
    let cols: Vec<String> = first.keys().cloned().collect();
    for it in items {
        let obj = it.as_object()?;
        if obj.len() != cols.len() || cols.iter().any(|c| !obj.contains_key(c)) {
            return None;
        }
    }
    Some(cols)
}

/// A scalar as a cell: strings without their quotes, null as a dash,
/// everything else as JSON writes it.
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
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
