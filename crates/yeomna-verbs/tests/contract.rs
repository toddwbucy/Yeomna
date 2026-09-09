//! The contract pinned: every variant round-trips, every wire name matches
//! the R4 table, strangers and smuggled actors die at the boundary.

use serde_json::json;
use yeomna_verbs::*;

/// One example of every verb. `Verb::wire_name`'s exhaustive match forces
/// an edit there when a variant is added, and the comment there points
/// here: extend this list and the count below in the same change.
fn all_examples() -> Vec<Verb> {
    vec![
        Verb::Orient(OrientRequest {
            graph: Some("yeomna".into()),
        }),
        Verb::Status(Empty {}),
        Verb::Health(Empty {}),
        Verb::Check(CheckRequest { key: "docA".into() }),
        Verb::Stats(StatsRequest { graph: None }),
        Verb::CodebaseStats(GraphScoped {
            graph: "yeomna".into(),
        }),
        Verb::Query(QueryRequest {
            search_text: "recursive cte".into(),
            limit: 10,
            kind: None,
            hybrid: true,
            structural: false,
        }),
        Verb::Get(KindKey {
            kind: "callable".into(),
            key: "src_lib_rs__Config__new__086f8847".into(),
        }),
        Verb::List(ListRequest {
            kind: Some("document".into()),
            limit: 20,
            offset: 0,
            parent: None,
        }),
        Verb::Count(CountRequest { kind: None }),
        Verb::Recent(RecentRequest { limit: 10 }),
        Verb::Insert(WriteRequest {
            kind: "document".into(),
            key: "docA".into(),
            payload: json!({"title": "t"}),
        }),
        Verb::Update(WriteRequest {
            kind: "document".into(),
            key: "docA".into(),
            payload: json!({"title": "u"}),
        }),
        Verb::Delete(KindKey {
            kind: "document".into(),
            key: "docA".into(),
        }),
        Verb::Purge(PurgeRequest {
            key: "docA".into(),
            force: true,
        }),
        Verb::GraphTraverse(TraverseRequest {
            graph: "yeomna".into(),
            start: "a".into(),
            relations: vec!["calls".into()],
            bases: vec!["declared".into(), "structural".into()],
            depth: 20,
            limit: 10_000,
        }),
        Verb::GraphNeighbors(NeighborsRequest {
            graph: "yeomna".into(),
            key: "a".into(),
            direction: Direction::Both,
            relations: vec![],
            bases: vec![],
            limit: 20,
        }),
        Verb::GraphShortestPath(ShortestPathRequest {
            graph: "yeomna".into(),
            from: "a".into(),
            to: "b".into(),
            relations: vec![],
            bases: vec![],
            cap: 10_000,
        }),
        Verb::GraphList(Empty {}),
        Verb::GraphCreate(GraphName {
            name: "scratch".into(),
        }),
        Verb::GraphDrop(DropRequest {
            name: "scratch".into(),
            force: true,
        }),
        Verb::GraphMaterialize(GraphScoped {
            graph: "yeomna".into(),
        }),
        Verb::SchemaApply(SchemaApplyRequest { database: None }),
        Verb::SchemaList(Empty {}),
        Verb::SchemaShow(Empty {}),
        Verb::SchemaVersion(Empty {}),
        Verb::DatabaseList(Empty {}),
        Verb::DatabaseCreate(DatabaseCreateRequest {
            name: "sidecar".into(),
            kind: DatabaseKind::Plain,
        }),
        Verb::DatabaseDrop(DropRequest {
            name: "sidecar".into(),
            force: true,
        }),
        Verb::Sql(SqlRequest {
            database: "sidecar".into(),
            statement: "SELECT 1".into(),
        }),
        Verb::EmbedText(EmbedTextRequest {
            text: "hello".into(),
            task: None,
        }),
        Verb::GraphEmbedEmbed(GraphKey {
            graph: "yeomna".into(),
            key: "a".into(),
        }),
        Verb::GraphEmbedNeighbors(GraphEmbedNeighborsRequest {
            graph: "yeomna".into(),
            key: "a".into(),
            limit: 10,
        }),
        Verb::GraphEmbedUpdate(GraphEmbedUpdateRequest {
            graph: "yeomna".into(),
            scope: UpdateScope::Stale,
        }),
        Verb::Ingest(IngestRequest {
            path: "/corpus".into(),
            graph: "yeomna".into(),
            overwrite: true,
        }),
        Verb::CodebaseIngest(IngestRequest {
            path: "/repo".into(),
            graph: "yeomna".into(),
            overwrite: false,
        }),
        Verb::CodebaseRetire(RetireRequest {
            graph: "yeomna".into(),
            prefix: "src/old/".into(),
            force: true,
        }),
        Verb::CodebasePrune(DropScoped {
            graph: "yeomna".into(),
            force: true,
        }),
        Verb::CodebaseDrift(DriftRequest {
            graph: "yeomna".into(),
            path: "/repo".into(),
        }),
        Verb::CodebaseValidate(GraphScoped {
            graph: "yeomna".into(),
        }),
        Verb::EdgeAssert(EdgeAssertRequest {
            from: "frontend_db".into(),
            to: "redis".into(),
            relation: "depends_on".into(),
            payload: json!({"since": "2026-09"}),
        }),
        Verb::EdgeRetract(EdgeRetractRequest {
            from: "frontend_db".into(),
            to: "redis".into(),
            relation: "depends_on".into(),
        }),
    ]
}

/// The R4 table, as the test sees it, extended by R18 (spec 014): the
/// two edge verbs land at the end so every earlier position is stable.
const WIRE_NAMES: [&str; 42] = [
    "orient",
    "status",
    "health",
    "check",
    "stats",
    "codebase.stats",
    "query",
    "get",
    "list",
    "count",
    "recent",
    "insert",
    "update",
    "delete",
    "purge",
    "graph.traverse",
    "graph.neighbors",
    "graph.shortest-path",
    "graph.list",
    "graph.create",
    "graph.drop",
    "graph.materialize",
    "schema.apply",
    "schema.list",
    "schema.show",
    "schema.version",
    "database.list",
    "database.create",
    "database.drop",
    "sql",
    "embed.text",
    "graph-embed.embed",
    "graph-embed.neighbors",
    "graph-embed.update",
    "ingest",
    "codebase.ingest",
    "codebase.retire",
    "codebase.prune",
    "codebase.drift",
    "codebase.validate",
    "edge.assert",
    "edge.retract",
];

#[test]
fn every_verb_round_trips_and_names_match_the_r4_table() {
    let examples = all_examples();
    assert_eq!(examples.len(), 42, "one example per verb");
    let mut seen = std::collections::BTreeSet::new();
    for (verb, expected_name) in examples.iter().zip(WIRE_NAMES) {
        assert_eq!(verb.wire_name(), expected_name, "R4 table order");
        assert!(seen.insert(verb.wire_name()), "duplicate wire name");
        let wire = serde_json::to_value(verb).unwrap();
        assert_eq!(wire["verb"], expected_name, "serde tag matches");
        let back: Verb = serde_json::from_value(wire).unwrap();
        assert_eq!(&back, verb, "round-trip identity");
    }
}

#[test]
fn wire_names_carry_no_mythology() {
    for name in WIRE_NAMES {
        let lower = name.to_lowercase();
        for banned in ["db.", "aql", "arango", "collection", "hades", "persephone"] {
            assert!(!lower.contains(banned), "{name} carries {banned}");
        }
    }
}

#[test]
fn ec1_unknown_verb_is_an_error_naming_the_stranger() {
    let err = serde_json::from_value::<Verb>(json!({
        "verb": "db.aql", "args": {}
    }))
    .unwrap_err();
    assert!(err.to_string().contains("db.aql"), "err: {err}");
}

#[test]
fn ec2_a_smuggled_actor_dies_at_the_boundary() {
    // Inside args, on a request with fields.
    let err = serde_json::from_value::<Verb>(json!({
        "verb": "purge",
        "args": {"key": "docA", "force": true, "actor": "root"}
    }))
    .unwrap_err();
    assert!(err.to_string().contains("actor"), "err: {err}");
    // Inside args, on an empty request.
    serde_json::from_value::<Verb>(json!({
        "verb": "status", "args": {"actor": "root"}
    }))
    .unwrap_err();
    // Beside args, at the envelope level. Serde's adjacent tagging accepts
    // unknown siblings unless the enum denies them, so this arm guards a
    // real hole rather than a hypothetical one.
    for smuggled in [
        json!({"verb": "status", "args": {}, "actor": "root"}),
        json!({"verb": "purge", "args": {"key": "docA"}, "actor": "root"}),
    ] {
        let err = serde_json::from_value::<Verb>(smuggled).unwrap_err();
        assert!(err.to_string().contains("actor"), "err: {err}");
    }
}

#[test]
fn envelope_shapes_match_the_capture() {
    let ok = envelope("graph.traverse", json!({"nodes": []}));
    let v = serde_json::to_value(&ok).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["command"], "graph.traverse");
    assert!(v["data"].is_object());
    assert!(v.get("error").is_none(), "no error key on success");
    // RFC 3339 parses back.
    chrono::DateTime::parse_from_rfc3339(v["timestamp"].as_str().unwrap()).unwrap();

    let err = error_envelope("purge", &VerbError::NotFound("docA".into()));
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v["success"], false);
    assert_eq!(v["error"], "not-found: docA");
    assert!(v.get("data").is_none(), "no data key on failure");
}

#[test]
fn error_kinds_are_stable_wire_strings() {
    let cases: [(VerbError, &str); 5] = [
        (VerbError::NotFound("x".into()), "not-found"),
        (VerbError::InvalidArgs("x".into()), "invalid-args"),
        (VerbError::Unimplemented("x".into()), "unimplemented"),
        (VerbError::Denied("x".into()), "denied"),
        (VerbError::Internal("x".into()), "internal"),
    ];
    for (e, kind) in cases {
        assert_eq!(e.kind(), kind);
        assert!(e.to_string().starts_with(kind), "Display leads with kind");
        let wire = serde_json::to_value(&e).unwrap();
        assert_eq!(wire["kind"], kind, "serde tag matches kind()");
    }
}

#[test]
fn ec3_sql_against_the_kg_database_is_representable() {
    // The refusal is runtime policy (Phase 4), not a type-level hole.
    let v: Verb = serde_json::from_value(json!({
        "verb": "sql",
        "args": {"database": "yeomna", "statement": "SELECT 1"}
    }))
    .unwrap();
    assert_eq!(v.wire_name(), "sql");
}

#[test]
fn defaults_match_the_store_prd() {
    let t: Verb = serde_json::from_value(json!({
        "verb": "graph.traverse",
        "args": {"graph": "g", "start": "a"}
    }))
    .unwrap();
    let Verb::GraphTraverse(req) = t else {
        panic!("wrong variant")
    };
    assert_eq!(req.depth, 20, "reference-observed depth ceiling");
    assert_eq!(req.limit, 10_000, "the hard row cap");
    assert!(req.relations.is_empty() && req.bases.is_empty());
}
