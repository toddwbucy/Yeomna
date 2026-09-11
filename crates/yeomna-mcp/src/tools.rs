//! The tool list, derived from the contract and never written beside it
//! (spec 024 D5).
//!
//! `schema_for!(Verb)` renders the adjacently-tagged enum as a `oneOf`
//! with one entry per variant. Each entry carries the variant's doc
//! comment as its `description` and the wire name as a `const`, and each
//! request struct's field doc comments arrive as property descriptions.
//! So the 42 doc comments on `Verb` become the tool documentation that
//! has never reached a caller.
//!
//! **The shape this depends on was measured before it was relied on**
//! (2026-09-10, schemars 1), and one part of it is not obvious: `args` is
//! a `$ref` into a `$defs` map at the root of the whole enum schema, not
//! an inlined subschema. A tool whose `inputSchema` is a bare `$ref` with
//! nowhere to resolve is still a well-formed tool object and still counts
//! toward the census, so the count test cannot see that mistake and
//! `refs_resolve` has to.

use serde_json::{Map, Value, json};
use yeomna_verbs::{Verb, WIRE_NAMES};

/// Wire names kept out of the model-controlled surface.
///
/// **R29, ruled by Todd 2026-09-11: tool abstractions only.** MCP defines
/// tools as model-controlled, meaning the model discovers and invokes
/// them from context, so the difference from the CLI is authorship rather
/// than permission: a person who types a statement decided those bytes,
/// while a model composes them and leaves an audit row whose actor is
/// true and whose authorship has thinned. The charter sentence honored is
/// section 6's "Nobody writes SQL".
///
/// The ruling does not rest on blast radius. R17a already refuses the
/// appliance's own footing, templates, and any kg-pattern database by
/// catalog probe, under a role holding no grant on any KG table, so the
/// corpus is structurally unreachable through `sql` regardless. The verb
/// itself is untouched and stays reachable through `yeomna call` and the
/// CLI tree.
///
/// It is a principle and not a one-off: a future verb that hands raw text
/// to the engine belongs here without a new ruling.
pub const EXCLUDED: [&str; 1] = ["sql"];

/// One MCP tool definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
}

impl Tool {
    /// The wire form, as `tools/list` carries it.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

/// True when any `$ref` appears anywhere in the subtree, which is what
/// decides whether a tool needs the definitions carried with it.
fn contains_ref(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| k == "$ref" || contains_ref(x)),
        Value::Array(a) => a.iter().any(contains_ref),
        _ => false,
    }
}

/// The definition a `#/$defs/<name>` pointer names, if it is that shape.
fn resolve<'a>(reference: &str, defs: &'a Map<String, Value>) -> Option<&'a Value> {
    defs.get(reference.strip_prefix("#/$defs/")?)
}

/// The 41 tools, in `WIRE_NAMES` order, with `EXCLUDED` filtered out.
///
/// The filter is over `WIRE_NAMES`, which is why an exclusion costs one
/// line rather than a second table: the contract stays the only list.
///
/// **The order is `WIRE_NAMES`, not the enum's declaration order, and
/// they differ.** `Verb` groups the edge verbs with the phase that added
/// them (R18, Phase 4) while the R4 table lists them last, so walking the
/// schema's `oneOf` and stopping there would produce a deterministic
/// order that is not the contract's. Both are stable, which is why this
/// is a correctness point rather than a cosmetic one: the census compares
/// against `WIRE_NAMES` and the table is what binds.
pub fn tools() -> &'static [Tool] {
    static TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(derive)
}

/// Walk the contract's schema once. `tools` memoizes this, because the
/// contract is compiled in and cannot change while the process runs, and
/// rebuilding 41 schemas on every call is work with no possible new
/// answer.
fn derive() -> Vec<Tool> {
    let root: Value = serde_json::to_value(schemars::schema_for!(Verb))
        .expect("the contract's schema serializes");
    let defs = root
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let variants = root
        .get("oneOf")
        .and_then(Value::as_array)
        .expect("an adjacently-tagged enum renders as oneOf, measured 2026-09-10");

    let mut out = Vec::new();
    for entry in variants {
        let name = entry
            .pointer("/properties/verb/const")
            .and_then(Value::as_str)
            .expect("every variant carries its wire name as a const");
        // Borrow the contract's own &'static str so a tool name cannot
        // drift from the wire name by way of an allocation.
        let Some(name) = WIRE_NAMES.iter().find(|w| **w == name) else {
            panic!("the schema named {name:?}, which is not a wire name");
        };
        if EXCLUDED.contains(name) {
            continue;
        }
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let args = entry
            .pointer("/properties/args")
            .expect("every variant carries args");
        let mut schema = match args.get("$ref").and_then(Value::as_str) {
            Some(r) => resolve(r, &defs)
                .unwrap_or_else(|| panic!("{name}: args names {r}, which $defs does not define"))
                .clone(),
            // An inlined schema is legal too, and costs nothing to accept.
            None => args.clone(),
        };
        // Carry the definitions only when something points at them, so a
        // verb taking nothing keeps the exact empty-object shape the
        // revision recommends rather than growing a `$defs` it never uses.
        if contains_ref(&schema)
            && let Some(m) = schema.as_object_mut()
        {
            m.insert("$defs".into(), Value::Object(defs.clone()));
        }
        out.push(Tool {
            name,
            description,
            input_schema: schema,
        });
    }

    // Re-order to the contract's table. Every entry is found, because the
    // loop above already refused any name the table does not carry.
    let mut ordered = Vec::with_capacity(out.len());
    for wire in WIRE_NAMES {
        if EXCLUDED.contains(&wire) {
            continue;
        }
        let i = out
            .iter()
            .position(|t| t.name == wire)
            .unwrap_or_else(|| panic!("{wire} is in the table and not in the schema"));
        ordered.push(out.remove(i));
    }
    debug_assert!(out.is_empty(), "the schema named a verb the table does not");
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_census_names_the_absence_rather_than_counting_to_it() {
        // FR2a. A count alone passes if some other verb went missing
        // while the excluded one was present, which is the identity
        // problem a count cannot see.
        let got: Vec<&str> = tools().iter().map(|t| t.name).collect();
        let want: Vec<&str> = WIRE_NAMES
            .iter()
            .copied()
            .filter(|w| !EXCLUDED.contains(w))
            .collect();
        assert_eq!(got, want, "the tool list is the contract minus EXCLUDED");
        for excluded in EXCLUDED {
            assert!(
                !got.contains(&excluded),
                "{excluded} is on a model-controlled surface (R29)"
            );
        }
        assert_eq!(got.len(), WIRE_NAMES.len() - EXCLUDED.len());
    }

    #[test]
    fn every_tool_carries_a_description_from_the_contract() {
        // FR2. True today because all 42 variants are documented. The
        // job of this test is to fail the day one is not.
        for t in tools() {
            assert!(
                !t.description.trim().is_empty(),
                "{} has no description, so its doc comment is missing",
                t.name
            );
        }
    }

    #[test]
    fn a_verb_taking_nothing_is_an_empty_object_and_not_null() {
        // FR3, and the shape the revision recommends for no parameters:
        // an object that explicitly accepts only empty objects. The
        // schema also carries `Empty`'s own doc comment as a description,
        // which is additional rather than contrary, so the assertion is
        // on the substance rather than on exact equality.
        let status = tools().iter().find(|t| t.name == "status").unwrap();
        let s = &status.input_schema;
        assert!(!s.is_null(), "an inputSchema is never null");
        assert_eq!(s.get("type"), Some(&json!("object")));
        assert_eq!(s.get("additionalProperties"), Some(&json!(false)));
        assert!(
            s.get("properties")
                .is_none_or(|p| p.as_object().is_some_and(serde_json::Map::is_empty)),
            "a verb taking nothing declares no properties"
        );
    }

    #[test]
    fn the_order_is_the_contract_table_and_not_the_enums_declaration_order() {
        // These differ: `Verb` groups the edge verbs with Phase 4 while
        // the R4 table lists them last. Both are deterministic, so only
        // a test can say which one shipped.
        let got: Vec<&str> = tools().iter().map(|t| t.name).collect();
        let edge = got.iter().position(|n| *n == "edge.assert").unwrap();
        let purge = got.iter().position(|n| *n == "purge").unwrap();
        assert!(
            edge > purge + 1,
            "the edge verbs follow the table's order, not the enum's"
        );
        assert_eq!(got.last(), Some(&"edge.retract"));
    }

    #[test]
    fn every_ref_resolves_inside_its_own_tool() {
        // FR3a. The failure this guards is invisible to the census: a
        // dangling reference is still a well-formed tool object.
        fn refs(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    for (k, x) in m {
                        if k == "$ref"
                            && let Some(s) = x.as_str()
                        {
                            out.push(s.to_string());
                        }
                        refs(x, out);
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
                _ => {}
            }
        }
        for t in tools() {
            let mut found = Vec::new();
            refs(&t.input_schema, &mut found);
            let defs = t.input_schema.get("$defs").and_then(Value::as_object);
            for r in found {
                let target = r
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("{}: {r} is not a local definition pointer", t.name));
                assert!(
                    defs.is_some_and(|d| d.contains_key(target)),
                    "{}: {r} resolves to nothing inside this tool",
                    t.name
                );
            }
        }
    }

    #[test]
    fn a_schema_is_an_object_schema_and_never_null() {
        for t in tools() {
            assert_eq!(
                t.input_schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{} has a non-object input schema",
                t.name
            );
        }
    }

    #[test]
    fn field_doc_comments_reach_the_caller_as_property_descriptions() {
        // The point of deriving rather than writing: the contract's own
        // prose is what a caller reads.
        let t = tools().iter().find(|t| t.name == "graph.traverse").unwrap();
        let d = t
            .input_schema
            .pointer("/properties/relations/description")
            .and_then(Value::as_str)
            .expect("relations carries its doc comment");
        assert!(d.contains("Relation filter"), "got {d:?}");
    }
}
