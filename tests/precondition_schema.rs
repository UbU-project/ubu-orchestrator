use serde_json::{json, Value};
use std::collections::BTreeSet;
use ubu_core::core::{InstanceMode, UniverseState};
use ubu_orchestrator::services::precondition_advisor::{self as advisor, Context, DescribedTask};

const FACT: &str = "facts.synthetic.ready";
const NUMBER: &str = "numeric_values.synthetic.charge";
const SET: &str = "set_memberships.synthetic.tools";
const EVENT: &str = "event_markers.synthetic.inspection";

fn context(targets: &[&str]) -> Context {
    Context {
        tasks: vec![DescribedTask {
            id: "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70".into(),
            title: "Synthetic inspection".into(),
            description: None,
        }],
        targets: targets.iter().map(|s| (*s).into()).collect(),
    }
}

// Dependency-free interpretation of the JSON Schema keywords used by this
// grammar. This knows no predicates, collections or precondition semantics;
// it evaluates the built artifact independently of the production validator.
fn admits(schema: &Value, root: &Value, value: &Value) -> bool {
    let object = schema.as_object().expect("schema object");
    for key in object.keys() {
        assert!(
            [
                "$ref",
                "$defs",
                "oneOf",
                "type",
                "const",
                "enum",
                "required",
                "properties",
                "additionalProperties",
                "minItems",
                "maxItems",
                "items"
            ]
            .contains(&key.as_str()),
            "unsupported JSON Schema keyword {key}"
        );
    }
    if let Some(reference) = schema["$ref"].as_str() {
        return admits(
            root.pointer(reference.strip_prefix('#').unwrap())
                .expect("local reference resolves"),
            root,
            value,
        );
    }
    if let Some(branches) = schema["oneOf"].as_array() {
        if branches
            .iter()
            .filter(|branch| admits(branch, root, value))
            .count()
            != 1
        {
            return false;
        }
    }
    if let Some(expected) = schema.get("const") {
        if value != expected {
            return false;
        }
    }
    if let Some(options) = schema["enum"].as_array() {
        if !options.contains(value) {
            return false;
        }
    }
    if let Some(types) = schema.get("type") {
        let matches = |kind: &Value| match kind.as_str().unwrap() {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            other => panic!("unsupported type {other}"),
        };
        if !types
            .as_array()
            .map_or_else(|| matches(types), |types| types.iter().any(matches))
        {
            return false;
        }
    }
    if let Some(value) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            if required
                .iter()
                .any(|key| !value.contains_key(key.as_str().unwrap()))
            {
                return false;
            }
        }
        let properties = schema["properties"].as_object();
        for (key, value) in value {
            match properties.and_then(|properties| properties.get(key)) {
                Some(property) if !admits(property, root, value) => return false,
                None if schema["additionalProperties"] == false => return false,
                _ => {}
            }
        }
    }
    if let Some(values) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|n| values.len() < n as usize)
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|n| values.len() > n as usize)
        {
            return false;
        }
        if let Some(items) = schema.get("items") {
            if values.iter().any(|value| !admits(items, root, value)) {
                return false;
            }
        }
    }
    true
}

fn predicates(schema: &Value) -> BTreeSet<String> {
    schema["$defs"]["leaf"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|branch| {
            let predicate = &branch["properties"]["predicate"];
            predicate["enum"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![predicate["const"].clone()])
        })
        .map(|value| value.as_str().unwrap().into())
        .collect()
}

#[test]
fn facts_only_format_offers_exactly_equals_and_absent() {
    let schema = advisor::response_schema(&context(&[FACT])).unwrap();
    assert_eq!(
        predicates(&schema),
        ["equals".into(), "absent".into()].into()
    );
    assert_eq!(
        schema["$defs"]["leaf"]["oneOf"].as_array().unwrap().len(),
        2
    );
    assert!(advisor::response_schema(&context(&[])).is_none());
    let partitions = advisor::partition_targets(&[
        FACT.into(),
        NUMBER.into(),
        SET.into(),
        EVENT.into(),
        "facts.private prose".into(),
    ]);
    assert_eq!(partitions.all.len(), 4);
    assert_eq!(partitions.numbers, vec![NUMBER]);
    assert_eq!(partitions.memberships, vec![SET]);
}

#[test]
fn leaf_branches_enforce_expected_presence_kind_and_target_collection() {
    let schema = advisor::response_schema(&context(&[FACT, NUMBER, SET, EVENT])).unwrap();
    let branches = schema["$defs"]["leaf"]["oneOf"].as_array().unwrap();
    assert!(branches[0]["properties"].get("expected").is_none());
    assert_eq!(branches[0]["additionalProperties"], false);
    for branch in &branches[1..3] {
        assert!(branch["required"]
            .as_array()
            .unwrap()
            .contains(&json!("expected")));
        assert_eq!(
            branch["properties"]["expected"],
            json!({"type":["string","number","boolean"]})
        );
    }
    assert_eq!(branches[2]["properties"]["target"]["enum"], json!([SET]));
    assert_eq!(branches[3]["properties"]["target"]["enum"], json!([NUMBER]));
    assert_eq!(
        branches[3]["properties"]["expected"],
        json!({"type":"number"})
    );
    assert_eq!(predicates(&schema).len(), 7);
}

#[test]
fn representative_trees_test_schema_and_validator_with_the_approved_subset_contract() {
    let schema = advisor::response_schema(&context(&[FACT, NUMBER, SET, EVENT])).unwrap();
    let mut state = UniverseState::new(
        ubu_core::UbuTimestamp::parse("2026-10-05T08:00:00Z").unwrap(),
        "Synthetic schema contract",
    );
    state.facts.insert("synthetic.ready".into(), json!(true));
    state.numeric_values.insert("synthetic.charge".into(), 30.0);
    state
        .set_memberships
        .insert("synthetic.tools".into(), Default::default());
    state
        .event_markers
        .insert("synthetic.inspection".into(), vec![]);
    let leaf = json!({"target":FACT,"predicate":"equals","expected":true});
    let mut cases = vec![
        ("equals scalar", leaf.clone(), true, true),
        (
            "absent",
            json!({"target":FACT,"predicate":"absent"}),
            true,
            true,
        ),
        (
            "equals without expected",
            json!({"target":FACT,"predicate":"equals"}),
            false,
            false,
        ),
        (
            "comparison over facts",
            json!({"target":FACT,"predicate":"at_least","expected":3}),
            false,
            false,
        ),
        (
            "absent with expected",
            json!({"target":FACT,"predicate":"absent","expected":true}),
            false,
            false,
        ),
        (
            "null expected",
            json!({"target":FACT,"predicate":"equals","expected":null}),
            false,
            false,
        ),
        (
            "numeric string",
            json!({"target":NUMBER,"predicate":"at_least","expected":"invented"}),
            false,
            false,
        ),
        (
            "membership over facts",
            json!({"target":FACT,"predicate":"member_of","expected":true}),
            false,
            false,
        ),
        (
            "membership object",
            json!({"target":SET,"predicate":"member_of","expected":{}}),
            false,
            false,
        ),
        (
            "membership array",
            json!({"target":SET,"predicate":"member_of","expected":[]}),
            false,
            false,
        ),
        ("empty group", json!({"all_of":[]}), false, false),
        ("group not array", json!({"any_of":{}}), false, false),
        (
            "mixed group",
            json!({"all_of":[leaf],"any_of":[leaf]}),
            false,
            false,
        ),
        (
            "unknown key",
            json!({"target":FACT,"predicate":"equals","expected":true,"invented":true}),
            false,
            false,
        ),
        (
            "unknown predicate",
            json!({"target":FACT,"predicate":"invented","expected":true}),
            false,
            false,
        ),
        (
            "unknown collection",
            json!({"target":"invented.synthetic.ready","predicate":"equals","expected":true}),
            false,
            false,
        ),
        (
            "nonstring target",
            json!({"target":3,"predicate":"equals","expected":true}),
            false,
            false,
        ),
        (
            "hidden bad branch",
            json!({"any_of":[leaf,{"target":FACT,"predicate":"equals"}]}),
            false,
            false,
        ),
        (
            "groups of leaves",
            json!({"all_of":[{"any_of":[leaf]}]}),
            true,
            true,
        ),
        (
            "four levels",
            json!({"all_of":[{"any_of":[{"all_of":[leaf]}]}]}),
            false,
            true,
        ),
        (
            "equals object accepted by core",
            json!({"target":FACT,"predicate":"equals","expected":{}}),
            false,
            true,
        ),
        (
            "equals array accepted by core",
            json!({"target":FACT,"predicate":"equals","expected":[]}),
            false,
            true,
        ),
        (
            "eleven children within validator bounds",
            json!({"all_of":vec![leaf.clone();11]}),
            false,
            true,
        ),
        (
            "111 nodes",
            json!({"all_of":vec![json!({"any_of":vec![leaf.clone();10]});10]}),
            true,
            true,
        ),
        (
            "131 nodes",
            json!({"all_of":vec![json!({"any_of":vec![leaf.clone();12]});10]}),
            false,
            false,
        ),
    ];
    for predicate in ["at_least", "at_most", "greater_than", "less_than"] {
        cases.push((
            predicate,
            json!({"target":NUMBER,"predicate":predicate,"expected":25}),
            true,
            true,
        ));
    }
    for expected in [json!("invented"), json!(3), json!(true)] {
        cases.push((
            "membership scalar",
            json!({"target":SET,"predicate":"member_of","expected":expected}),
            true,
            true,
        ));
    }
    for (name, tree, schema_ok, validator_ok) in cases {
        let formatted = json!({"proposals":[{"id":context(&[]).tasks[0].id,"precondition":tree}]});
        let admitted = admits(&schema, &schema, &formatted);
        let valid = advisor::validate_tree(&tree, &state, InstanceMode::UserMode).is_ok();
        assert_eq!(admitted, schema_ok, "schema: {name}");
        assert_eq!(valid, validator_ok, "validator: {name}");
        assert!(
            !admitted || valid,
            "schema admitted a validator refusal: {name}"
        );
    }
    assert_eq!(advisor::SCHEMA_LEVELS, 3);
    assert_eq!(advisor::SCHEMA_GROUP_ITEMS, 10);
    assert_eq!(advisor::MAX_DEPTH, 16);
    assert_eq!(advisor::MAX_NODES, 128);
}
