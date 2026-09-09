//! The corpus's own graph notation (spec 015, R19a): fenced `graph`
//! blocks in Markdown declaring nodes and edges, and `conforms:` headers
//! in source naming the claims a file answers to. Both parse here, pure
//! and unit-tested, and `documents` turns the results into rows.
//!
//! Nothing here invents vocabulary. Relation names, kinds, and tags are
//! carried as written, checked for shape only, because a semantic KG's
//! language belongs to its corpus and not to this parser.

use std::collections::BTreeMap;

/// A node the corpus declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub key: String,
    /// The block's own `kind:`, preserved in payload and never mapped
    /// onto the schema's node kinds.
    pub kind: Option<String>,
    pub tag: Option<String>,
    /// Every other `key: value` line, verbatim.
    pub extra: BTreeMap<String, String>,
    pub line: usize,
}

/// An edge the corpus declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub relation: String,
    pub from: String,
    pub to: String,
    pub extra: BTreeMap<String, String>,
    pub line: usize,
}

/// A block or a header this parser declined, with the line and the
/// reason, so the summary can name it (EC-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub line: usize,
    pub reason: String,
}

/// Everything one document's graph blocks declared.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GraphBlocks {
    pub blocks_seen: usize,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub refusals: Vec<Refusal>,
}

/// The shape a declared or asserted relation must have, mirroring the
/// `edges_declared` and `edges_asserted` partition CHECKs by
/// construction: lowercase, then lowercase, digits, underscore, or
/// hyphen, at most 63 bytes. Hyphens are admitted because the first
/// corpus speaks kebab (`floor-link`), and sources speak their own words.
pub fn is_relation(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A node key as the notation writes it: one token, no whitespace,
/// printable, bounded.
fn is_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@' | '+')
        })
}

/// A `kind:` or `tag:` value: one lowercase word. The corpus carries one
/// `kind:` whose value is a prose sentence, and that block is refused
/// rather than half-read.
fn is_word(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A claim slug as `conforms:` headers write it.
fn is_slug(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.len() <= 200
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Parse every fenced `graph` block in a Markdown document.
pub fn parse_graph_blocks(markdown: &str) -> GraphBlocks {
    let mut out = GraphBlocks::default();
    let mut block: Option<(usize, Vec<(usize, String)>)> = None;
    let mut in_other_fence = false;
    for (i, raw) in markdown.lines().enumerate() {
        let line_no = i + 1;
        let t = raw.trim();
        if let Some((_, lines)) = block.as_mut() {
            if t == "```" {
                let (start, lines) = block.take().expect("checked above");
                out.blocks_seen += 1;
                parse_block(start, &lines, &mut out);
            } else {
                lines.push((line_no, raw.to_string()));
            }
            continue;
        }
        if in_other_fence {
            if t.starts_with("```") {
                in_other_fence = false;
            }
            continue;
        }
        if t == "```graph" || t.starts_with("```graph ") {
            block = Some((line_no, Vec::new()));
        } else if t.starts_with("```") {
            in_other_fence = true;
        }
    }
    if let Some((start, _)) = block {
        out.blocks_seen += 1;
        out.refusals.push(Refusal {
            line: start,
            reason: "graph block never closed".into(),
        });
    }
    out
}

enum Stanza {
    Node(GraphNode),
    Edge(GraphEdge),
}

/// Keys the notation reserves, refused as extras rather than stored, so
/// a missing blank line between a node stanza and an edge stanza cannot
/// fold one into the other's extras and vanish, and so an extra can never
/// shadow a field the writer sets on the row. Two sets, because the
/// notation lets an edge carry `tag:` and `kind:` as attributes (the
/// corpus's own format document does), while on a node those are the
/// declaration itself.
const ROW_FIELDS: [&str; 5] = [
    "_key",
    "declared_in",
    "declared_at_line",
    "placeholder",
    "first_named_in",
];
const NODE_RESERVED: [&str; 4] = ["node", "edge", "from", "to"];
const EDGE_RESERVED: [&str; 2] = ["node", "edge"];

fn is_reserved(key: &str, stanza: &[&str]) -> bool {
    ROW_FIELDS.contains(&key) || stanza.contains(&key)
}

fn reserved(line: usize, key: &str) -> Refusal {
    Refusal {
        line,
        reason: format!(
            "{key}: is reserved here, a blank line separates stanzas and a row field is not an extra"
        ),
    }
}

/// One block: stanzas split on blank lines, any bad stanza refuses the
/// whole block (EC-7), because a block is one declaration.
fn parse_block(start: usize, lines: &[(usize, String)], out: &mut GraphBlocks) {
    let mut stanzas: Vec<Vec<(usize, &str)>> = Vec::new();
    let mut current: Vec<(usize, &str)> = Vec::new();
    for (n, l) in lines {
        let t = l.trim();
        if t.is_empty() {
            if !current.is_empty() {
                stanzas.push(std::mem::take(&mut current));
            }
        } else {
            current.push((*n, t));
        }
    }
    if !current.is_empty() {
        stanzas.push(current);
    }
    if stanzas.is_empty() {
        out.refusals.push(Refusal {
            line: start,
            reason: "empty graph block".into(),
        });
        return;
    }
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for stanza in stanzas {
        match parse_stanza(&stanza) {
            Ok(Stanza::Node(n)) => nodes.push(n),
            Ok(Stanza::Edge(e)) => edges.push(e),
            Err(r) => {
                out.refusals.push(r);
                return;
            }
        }
    }
    out.nodes.extend(nodes);
    out.edges.extend(edges);
}

fn parse_stanza(lines: &[(usize, &str)]) -> Result<Stanza, Refusal> {
    let mut fields: Vec<(usize, String, String)> = Vec::with_capacity(lines.len());
    for (n, l) in lines {
        let Some((k, v)) = l.split_once(':') else {
            return Err(Refusal {
                line: *n,
                reason: format!("expected key: value, got {l:?}"),
            });
        };
        fields.push((*n, k.trim().to_string(), v.trim().to_string()));
    }
    let (first_line, first_key, first_val) = fields[0].clone();
    match first_key.as_str() {
        "node" => {
            if !is_key(&first_val) {
                return Err(Refusal {
                    line: first_line,
                    reason: format!("node key {first_val:?} is not one token"),
                });
            }
            let mut node = GraphNode {
                key: first_val,
                kind: None,
                tag: None,
                extra: BTreeMap::new(),
                line: first_line,
            };
            for (n, k, v) in fields.into_iter().skip(1) {
                match k.as_str() {
                    "kind" | "tag" => {
                        if !is_word(&v) {
                            return Err(Refusal {
                                line: n,
                                reason: format!("{k}: {v:?} is not one lowercase word"),
                            });
                        }
                        if k == "kind" {
                            node.kind = Some(v);
                        } else {
                            node.tag = Some(v);
                        }
                    }
                    _ => {
                        if is_reserved(&k, &NODE_RESERVED) {
                            return Err(reserved(n, &k));
                        }
                        node.extra.insert(k, v);
                    }
                }
            }
            Ok(Stanza::Node(node))
        }
        "edge" => {
            if !is_relation(&first_val) {
                return Err(Refusal {
                    line: first_line,
                    reason: format!("relation {first_val:?} is not identifier-shaped"),
                });
            }
            let mut from = None;
            let mut to = None;
            let mut extra = BTreeMap::new();
            for (n, k, v) in fields.into_iter().skip(1) {
                match k.as_str() {
                    "from" | "to" => {
                        if !is_key(&v) {
                            return Err(Refusal {
                                line: n,
                                reason: format!("{k}: {v:?} is not one token"),
                            });
                        }
                        let slot = if k == "from" { &mut from } else { &mut to };
                        if slot.is_some() {
                            return Err(reserved(n, &k));
                        }
                        *slot = Some(v);
                    }
                    _ => {
                        if is_reserved(&k, &EDGE_RESERVED) {
                            return Err(reserved(n, &k));
                        }
                        extra.insert(k, v);
                    }
                }
            }
            let (Some(from), Some(to)) = (from, to) else {
                return Err(Refusal {
                    line: first_line,
                    reason: format!("edge {first_val:?} needs both from: and to:"),
                });
            };
            Ok(Stanza::Edge(GraphEdge {
                relation: first_val,
                from,
                to,
                extra,
                line: first_line,
            }))
        }
        other => Err(Refusal {
            line: first_line,
            reason: format!("a stanza starts with node: or edge:, not {other:?}"),
        }),
    }
}

/// One `conforms:` header in source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformsHeader {
    pub slug: String,
    pub line: usize,
}

/// Every header one source file carries.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConformsScan {
    pub headers: Vec<ConformsHeader>,
    pub malformed: Vec<Refusal>,
}

/// Scan a source file for `conforms: <slug>` inside a comment. The
/// comment leaders cover Rust doc and line comments plus the common
/// hash and dash styles, so a Python or SQL file can carry the same
/// header the day someone writes one.
pub fn scan_conforms(source: &str) -> ConformsScan {
    const LEADERS: [&str; 6] = ["//!", "///", "//", "#", "--", "*"];
    let mut out = ConformsScan::default();
    for (i, raw) in source.lines().enumerate() {
        let t = raw.trim_start();
        let Some(body) = LEADERS.iter().find_map(|l| t.strip_prefix(l)) else {
            continue;
        };
        let Some(rest) = body.trim_start().strip_prefix("conforms:") else {
            continue;
        };
        let line = i + 1;
        match rest.split_whitespace().next() {
            Some(slug) if is_slug(slug) => out.headers.push(ConformsHeader {
                slug: slug.to_string(),
                line,
            }),
            other => out.malformed.push(Refusal {
                line,
                reason: format!("conforms: needs a slug, got {:?}", other.unwrap_or("")),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Prose before.

```graph
node: types-denial-precedes-permission
kind: assertion
tag: perturbation

edge: asserts
from: weaver-types
to: types-denial-precedes-permission
```

```rust
node: not-a-declaration
```

```graph
node: weaver-types
kind: crate
owner: platform

edge: floor-link
from: weaver-types
to: weaver-traits
weight: 2
```
"#;

    #[test]
    fn blocks_declare_nodes_and_edges_verbatim() {
        let g = parse_graph_blocks(SAMPLE);
        assert_eq!(g.blocks_seen, 2, "the rust fence is not a graph block");
        assert!(g.refusals.is_empty(), "{:?}", g.refusals);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 2);
        let claim = &g.nodes[0];
        assert_eq!(claim.key, "types-denial-precedes-permission");
        assert_eq!(claim.kind.as_deref(), Some("assertion"));
        assert_eq!(claim.tag.as_deref(), Some("perturbation"));
        assert_eq!(g.nodes[1].extra["owner"], "platform", "extras ride along");
        assert_eq!(g.edges[0].relation, "asserts");
        assert_eq!(
            g.edges[1].relation, "floor-link",
            "kebab carried as written"
        );
        assert_eq!(g.edges[1].extra["weight"], "2");
    }

    #[test]
    fn a_malformed_line_refuses_its_whole_block_and_nothing_else() {
        let md = "```graph\nnode: good-one\nkind: assertion\n```\n\n```graph\nnode: bad-one\nkind: the SPU still decides nothing about what matters\n\nedge: asserts\nfrom: a\nto: bad-one\n```\n";
        let g = parse_graph_blocks(md);
        assert_eq!(g.blocks_seen, 2);
        assert_eq!(g.nodes.len(), 1, "the good block survives");
        assert!(
            g.edges.is_empty(),
            "the bad block's edge went with it (EC-7)"
        );
        assert_eq!(g.refusals.len(), 1);
        assert_eq!(g.refusals[0].line, 8);
        assert!(g.refusals[0].reason.contains("lowercase word"));
    }

    #[test]
    fn shape_checks_refuse_what_the_partition_would() {
        for bad in [
            "Floor-Link",
            "has space",
            "",
            "9lives",
            "a".repeat(64).as_str(),
        ] {
            assert!(!is_relation(bad), "{bad:?}");
        }
        for good in ["asserts", "floor-link", "depends_on", "a"] {
            assert!(is_relation(good), "{good:?}");
        }
        let md = "```graph\nedge: Asserts\nfrom: a\nto: b\n```\n";
        let g = parse_graph_blocks(md);
        assert_eq!(g.edges.len(), 0);
        assert_eq!(g.refusals.len(), 1);
        let md = "```graph\nedge: asserts\nfrom: a\n```\n";
        assert!(
            parse_graph_blocks(md).refusals[0]
                .reason
                .contains("both from: and to:")
        );
    }

    #[test]
    fn reserved_keys_refuse_rather_than_become_extras() {
        // A missing blank line would otherwise fold the edge into the
        // node's extras and drop it with no count.
        let md =
            "```graph\nnode: a-claim\nkind: assertion\nedge: asserts\nfrom: x\nto: a-claim\n```\n";
        let g = parse_graph_blocks(md);
        assert!(g.nodes.is_empty() && g.edges.is_empty());
        assert_eq!(g.refusals.len(), 1);
        assert!(
            g.refusals[0].reason.starts_with("edge: is reserved"),
            "{:?}",
            g.refusals
        );
        // An extra cannot shadow a row field.
        let md = "```graph\nnode: a-claim\n_key: other\n```\n";
        assert!(
            parse_graph_blocks(md).refusals[0]
                .reason
                .starts_with("_key: is reserved")
        );
        // A second from: is a mistake, not an override.
        let md = "```graph\nedge: asserts\nfrom: x\nfrom: y\nto: z\n```\n";
        assert!(
            parse_graph_blocks(md).refusals[0]
                .reason
                .starts_with("from: is reserved")
        );
        // An edge carries tag: and kind: as attributes, as the corpus's
        // format document itself does, so those are extras on an edge and
        // the declaration on a node.
        let md = "```graph\nedge: party\nfrom: x\nto: y\ntag: manifest\nkind: witness\n```\n";
        let g = parse_graph_blocks(md);
        assert!(g.refusals.is_empty(), "{:?}", g.refusals);
        assert_eq!(g.edges[0].extra["tag"], "manifest");
        assert_eq!(g.edges[0].extra["kind"], "witness");
    }

    #[test]
    fn an_unclosed_block_is_refused_not_swallowed() {
        let g = parse_graph_blocks("```graph\nnode: x\n");
        assert_eq!(g.blocks_seen, 1);
        assert!(g.nodes.is_empty());
        assert!(g.refusals[0].reason.contains("never closed"));
    }

    #[test]
    fn conforms_headers_scan_from_doc_and_line_comments() {
        let src = "//! conforms: types-denial-precedes-permission\n\
                   fn a() {}\n\
                   /// conforms: admin-answer-and-exit-status-agree extra words\n\
                   // conforms: Not-A-Slug\n\
                   # conforms: py-style-claim\n\
                   let s = \"conforms: inside-a-string\";\n";
        let scan = scan_conforms(src);
        let slugs: Vec<&str> = scan.headers.iter().map(|h| h.slug.as_str()).collect();
        assert_eq!(
            slugs,
            [
                "types-denial-precedes-permission",
                "admin-answer-and-exit-status-agree",
                "py-style-claim"
            ]
        );
        assert_eq!(scan.headers[1].line, 3);
        assert_eq!(scan.malformed.len(), 1, "the capitalized one");
        assert_eq!(scan.malformed[0].line, 4);
    }
}
