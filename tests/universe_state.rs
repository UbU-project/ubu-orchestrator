//! The UniverseState route, in-process. Every fact here is invented.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-06-10T15:00:00Z";
const SCHEMA: &str = "ubu.orchestrator.universe_state.v1";
const COLLECTIONS: [&str; 4] = ["facts", "numeric_values", "set_memberships", "event_markers"];

async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes)})),
    )
}
async fn read(state: &AppState) -> Value {
    let (status, body) = request(state, "GET", "/universe-state", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn edit(state: &AppState, mutations: Value) -> (StatusCode, Value) {
    request(
        state,
        "PATCH",
        "/universe-state",
        json!({"schema_version": SCHEMA, "mutations": mutations}),
    )
    .await
}
async fn edited(state: &AppState, mutations: Value) -> Value {
    let (status, body) = edit(state, mutations).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
/// Every stored UniverseState row: id, version, label and payload.
async fn rows(state: &AppState) -> Vec<(String, i64, String, String)> {
    sqlx::query_as(
        "SELECT id, version, compartment_label, payload_json FROM objects \
         WHERE object_type = 'UniverseState' ORDER BY created_at",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .unwrap()
}
/// A stored state as bootstrap leaves one: version 1, labelled `bootstrap`.
async fn admit_state(state: &AppState, facts: Value) -> String {
    let id = UbuId::new(ObjectType::UniverseState);
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            UbuTimestamp::parse(NOW).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::UniverseState.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "bootstrap".to_owned(),
            payload: json!({
                "id": id,
                "captured_at": NOW,
                "facts": facts,
                "source_summary": "synthetic stored UniverseState",
                "schema_version": "core/universe-state/0.1",
                "provenance": {"created_at": NOW, "authority_source": "user"}
            }),
            created_at: NOW.to_owned(),
            updated_at: NOW.to_owned(),
        },
    )
    .await
    .unwrap();
    id.to_string()
}
async fn admit_task(state: &AppState, title: &str, extra: Value) -> String {
    let id = UbuId::new(ObjectType::Task);
    let mut payload = json!({
        "id": id,
        "title": title,
        "status": "active",
        "duration_minutes": 10,
        "provenance": {"created_at": NOW, "authority_source": "user"}
    });
    for (key, value) in extra.as_object().unwrap() {
        payload[key.as_str()] = value.clone();
    }
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            UbuTimestamp::parse(NOW).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload,
            created_at: NOW.to_owned(),
            updated_at: NOW.to_owned(),
        },
    )
    .await
    .unwrap();
    id.to_string()
}
async fn blocked_task_ids(state: &AppState) -> Vec<String> {
    let (status, body) = request(state, "POST", "/planning/generate", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["blocked_tasks"]
        .as_array()
        .map(|tasks| {
            tasks
                .iter()
                .map(|task| task["task_id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_read_on_an_empty_store_is_the_synthesized_empty_state_and_stores_nothing() {
    let state = state().await;
    let body = read(&state).await;
    assert_eq!(body["schema_version"], SCHEMA);
    assert_eq!(body["version"], Value::Null);
    for collection in COLLECTIONS {
        assert_eq!(body[collection], json!({}), "{collection}");
    }
    assert_eq!(body["source_summary"], "empty UniverseState synthesized by orchestrator");
    assert_eq!(body["confidence_summary"], Value::Null);
    assert!(body["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(body["captured_at"].as_str().is_some());
    assert!(rows(&state).await.is_empty(), "a read must not create a state");
}

#[tokio::test]
async fn each_of_the_nine_operations_round_trips() {
    let state = state().await;
    let cases = [
        (
            json!({"operation":"set_fact","target":"facts.kettle.descaled","payload":true}),
            "facts",
            json!({"kettle.descaled": true}),
        ),
        (
            json!({"operation":"clear_fact","target":"facts.kettle.descaled"}),
            "facts",
            json!({}),
        ),
        (
            json!({"operation":"set_numeric","target":"numeric_values.shelf.lids","payload":0.7}),
            "numeric_values",
            json!({"shelf.lids": 0.7}),
        ),
        (
            json!({"operation":"clear_numeric","target":"numeric_values.shelf.lids"}),
            "numeric_values",
            json!({}),
        ),
        (
            json!({"operation":"increment_numeric","target":"numeric_values.shelf.jars","payload":5}),
            "numeric_values",
            json!({"shelf.jars": 5.0}),
        ),
        (
            json!({"operation":"decrement_numeric","target":"numeric_values.shelf.jars","payload":2}),
            "numeric_values",
            json!({"shelf.jars": 3.0}),
        ),
        (
            json!({"operation":"add_membership","target":"set_memberships.toolbox","payload":"spanner"}),
            "set_memberships",
            json!({"toolbox": ["spanner"]}),
        ),
        (
            json!({"operation":"remove_membership","target":"set_memberships.toolbox","payload":"spanner"}),
            "set_memberships",
            json!({}),
        ),
        (
            json!({"operation":"append_event_marker","target":"event_markers.kettle.boiled","payload":{"cups":2}}),
            "event_markers",
            json!({"kettle.boiled": [{"cups": 2}]}),
        ),
    ];
    for (mutation, collection, expected) in cases {
        let written = edited(&state, json!([mutation])).await;
        assert_eq!(written[collection], expected, "{mutation}");
        // What a later read returns is what the write returned.
        assert_eq!(read(&state).await, written, "{mutation}");
    }
    // One row, ten versions: the seed and one per edit.
    let rows = rows(&state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, 10);
}

#[tokio::test]
async fn two_writes_in_sequence_are_versions_two_and_three() {
    // On an empty store the first edit seeds version 1 and writes version 2.
    let empty = state().await;
    let first = edited(
        &empty,
        json!([{"operation":"set_fact","target":"facts.kettle.descaled","payload":true}]),
    )
    .await;
    let second = edited(
        &empty,
        json!([{"operation":"set_fact","target":"facts.kettle.filled","payload":false}]),
    )
    .await;
    assert_eq!((first["version"].as_i64(), second["version"].as_i64()), (Some(2), Some(3)));
    assert_eq!(first["id"], second["id"]);
    assert_eq!(
        second["facts"],
        json!({"kettle.descaled": true, "kettle.filled": false})
    );

    // On a store that already holds version 1, the same: 2, then 3, in its own row and label.
    let stored = state().await;
    let id = admit_state(&stored, json!({"lamp.bulb": "fitted"})).await;
    assert_eq!(read(&stored).await["version"], 1);
    let first = edited(
        &stored,
        json!([{"operation":"set_fact","target":"facts.lamp.shade","payload":"green"}]),
    )
    .await;
    let second = edited(
        &stored,
        json!([{"operation":"clear_fact","target":"facts.lamp.bulb"}]),
    )
    .await;
    assert_eq!((first["version"].as_i64(), second["version"].as_i64()), (Some(2), Some(3)));
    assert_eq!(second["facts"], json!({"lamp.shade": "green"}));
    let rows = rows(&stored).await;
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].0.as_str(), rows[0].1, rows[0].2.as_str()), (id.as_str(), 3, "bootstrap"));
}

#[tokio::test]
async fn a_refused_edit_writes_nothing_and_applies_no_part_of_itself() {
    let good = json!({"operation":"set_fact","target":"facts.kettle.descaled","payload":true});
    let refusals = [
        (json!({"operation":"polish_fact","target":"facts.kettle.descaled","payload":true}), "unknown operation `polish_fact`"),
        (json!({"operation":"set_fact","target":"kettle","payload":true}), "malformed target `kettle`"),
        (json!({"operation":"set_fact","target":"cupboard.kettle","payload":true}), "malformed target `cupboard.kettle`"),
        (json!({"operation":"set_fact","target":"facts..descaled","payload":true}), "malformed target `facts..descaled`"),
        (json!({"operation":"set_fact","target":"numeric_values.shelf.jars","payload":true}), "operation target must be in the facts collection"),
        (json!({"operation":"set_fact","target":"facts.kettle.descaled"}), "operation requires a payload"),
        (json!({"operation":"increment_numeric","target":"numeric_values.shelf.jars","payload":"three"}), "payload must be a JSON number"),
        (json!({"operation":"add_membership","target":"set_memberships.toolbox","payload":["spanner"]}), "payload must be a JSON scalar"),
        (json!({"operation":"append_event_marker","target":"event_markers.kettle.boiled","payload":2}), "append_event_marker payload must be a JSON object"),
        (json!({"operation":"set_numeric","target":"numeric_values.shelf.jars","payload":"three"}), "payload must be a JSON number"),
        (json!({"operation":"set_numeric","target":"numeric_values.shelf.jars"}), "operation requires a payload"),
        (json!({"operation":"set_numeric","target":"facts.shelf.jars","payload":3}), "operation target must be in the numeric_values collection"),
        (json!({"operation":"clear_numeric","target":"numeric_values.shelf.jars","payload":0}), "clear_numeric does not accept a payload"),
        (json!({"operation":"clear_numeric","target":"numeric_values.shelf.jars","provenance_kind":"measured"}), "clear_numeric does not accept a provenance kind"),
        (json!({"operation":"clear_fact","target":"facts.kettle.descaled","provenance_kind":"asserted"}), "clear_fact does not accept a provenance kind"),
    ];

    // On an empty store a refusal leaves it empty: no seed is written first.
    let empty = state().await;
    for (bad, message) in &refusals {
        // The good mutation comes first, so a partial application would show.
        let (status, body) = edit(&empty, json!([good, bad])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(body["diagnostics"][0]["code"], "universe_mutation_invalid", "{bad}");
        assert_eq!(body["diagnostics"][0]["message"], format!("mutation 1: {message}"), "{bad}");
    }
    assert!(rows(&empty).await.is_empty());

    // On a stored state a refusal leaves the row byte for byte as it was.
    let stored = state().await;
    admit_state(&stored, json!({"lamp.bulb": "fitted"})).await;
    let before = rows(&stored).await;
    for (bad, _) in &refusals {
        let (status, _) = edit(&stored, json!([good, bad])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(rows(&stored).await, before);
    assert_eq!(read(&stored).await["facts"], json!({"lamp.bulb": "fitted"}));
}

#[tokio::test]
async fn a_malformed_request_is_refused_before_any_mutation_is_read() {
    let state = state().await;
    let good = json!({"operation":"set_fact","target":"facts.kettle.descaled","payload":true});
    for (body, code) in [
        (json!({"mutations": [good]}), "missing_schema_version"),
        (json!({"schema_version": "synthetic-wrong", "mutations": [good]}), "unknown_schema_version"),
        (json!({"schema_version": SCHEMA, "mutations": []}), "universe_mutations_empty"),
    ] {
        let (status, answer) = request(&state, "PATCH", "/universe-state", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(answer["diagnostics"][0]["code"], code);
    }
    // A key the mutation type does not have is not a mutation.
    for body in [
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact","target":"facts.kettle.descaled","payload":true,"provenance":"measured"}]}),
        json!({"schema_version": SCHEMA, "mutations": [good], "compartment_label": "bootstrap"}),
        // `note` was accepted and stored nowhere until P1B-59. It is refused now, not dropped.
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact","target":"facts.kettle.descaled","payload":true,"note":"read off the invented dial"}]}),
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact","target":"facts.kettle.descaled","payload":true,"provenance_kind":"guessed"}]}),
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact","target":"facts.kettle.descaled","payload":true,"provenance_kind":{"kind":"measured","confidence":0.9}}]}),
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact"}]}),
        json!({"schema_version": SCHEMA}),
    ] {
        let (status, _) = request(&state, "PATCH", "/universe-state", body.clone()).await;
        assert!(status.is_client_error(), "{body}: {status}");
    }
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn the_manual_route_refuses_intrinsic_affect_even_in_user_mode() {
    let state = state().await;
    let (status, refused) = edit(
        &state,
        json!([{"operation":"increment_numeric","target":"numeric_values.affect.energy","payload":1.5}]),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(refused["diagnostics"][0]["code"], "universe_target_namespace_invalid");
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn all_reserved_first_segments_refuse_every_write_without_seeding_state() {
    let state = state().await;
    for segment in [
        "facts",
        "numeric_values",
        "set_memberships",
        "event_markers",
        "affect",
    ] {
        for (operation, collection, payload) in [
            ("set_fact", "facts", json!(true)),
            ("set_numeric", "numeric_values", json!(1)),
            ("increment_numeric", "numeric_values", json!(1)),
            ("decrement_numeric", "numeric_values", json!(1)),
            (
                "add_membership",
                "set_memberships",
                json!("invented-spanner"),
            ),
            (
                "append_event_marker",
                "event_markers",
                json!({"invented_boil": true}),
            ),
        ] {
            let target = format!("{collection}.{segment}.invented_kettle");
            let (status, body) = edit(
                &state,
                json!([{"operation":operation,"target":target,"payload":payload}]),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(
                body["diagnostics"][0]["code"],
                "universe_target_namespace_invalid"
            );
            let expected = if segment == "affect" {
                format!("Key segment `affect` is reserved for intrinsic affect, which organization_mode and worker_mode refuse. The target would be `{target}`; the collection comes from the panel, not the key.")
            } else {
                format!("Key segment `{segment}` names a collection; the collection comes from the panel, not the key. The target would be `{target}`.")
            };
            assert_eq!(body["diagnostics"][0]["message"], expected);
        }
    }
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn reserved_words_in_later_segments_and_other_subjects_remain_writable() {
    let state = state().await;
    for key in ["kettle.facts.note", "kettle.affect.note", "fact.kettle"] {
        let after = edited(&state, json!([
            {"operation":"set_fact","target":format!("facts.{key}"),"payload":true},
            {"operation":"set_numeric","target":format!("numeric_values.{key}"),"payload":3},
            {"operation":"add_membership","target":format!("set_memberships.{key}"),"payload":"invented-spanner"},
            {"operation":"append_event_marker","target":format!("event_markers.{key}"),"payload":{"invented_boil":true}}
        ])).await;
        for collection in COLLECTIONS {
            assert!(after[collection].get(key).is_some(), "{after}");
        }
    }
}

#[tokio::test]
async fn a_reserved_write_in_a_batch_leaves_the_existing_state_unchanged() {
    let state = state().await;
    admit_state(&state, json!({"kettle.descaled": true})).await;
    let before = rows(&state).await;
    let (status, _) = edit(
        &state,
        json!([
            {"operation":"set_fact","target":"facts.kettle.label","payload":"invented-copper"},
            {"operation":"set_numeric","target":"numeric_values.numeric_values.jars","payload":3}
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(rows(&state).await, before);
}

#[tokio::test]
async fn legacy_keys_remain_readable_offered_evaluable_and_removable() {
    use ubu_core::core::{evaluate_universe_precondition, UniversePrecondition};
    use ubu_orchestrator::services::{precondition_advisor, universe_state};
    let state = state().await;
    // Task effects deliberately keep core semantics and can stage legacy keys.
    let task = admit_task(&state, "Synthetic: stage an invented legacy kettle", json!({"effects":{"mutations":[
        {"operation":"set_fact","target":"facts.facts.kettle","payload":true},
        {"operation":"set_fact","target":"facts.affect.energy","payload":true},
        {"operation":"set_numeric","target":"numeric_values.numeric_values.jars","payload":3},
        {"operation":"add_membership","target":"set_memberships.set_memberships.toolbox","payload":"invented-spanner"},
        {"operation":"append_event_marker","target":"event_markers.event_markers.boil","payload":{"invented_boil":true}}
    ]}})).await;
    let (status, body) = request(
        &state,
        "POST",
        &format!("/task/{task}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["diagnostics"], json!([]));
    let before = rows(&state).await;
    assert_eq!(read(&state).await["facts"]["facts.kettle"], true);
    let (current, _) = universe_state::read(&state).await.unwrap();
    let targets = precondition_advisor::targets(&current);
    for (target, predicate, expected) in [
        ("facts.facts.kettle", "equals", json!(true)),
        ("facts.affect.energy", "equals", json!(true)),
        ("numeric_values.numeric_values.jars", "at_least", json!(3)),
        (
            "set_memberships.set_memberships.toolbox",
            "member_of",
            json!("invented-spanner"),
        ),
        (
            "event_markers.event_markers.boil",
            "equals",
            json!([{"invented_boil": true}]),
        ),
    ] {
        assert!(targets.contains(target), "{target}");
        let mut leaf = json!({"target":target,"predicate":predicate});
        if !expected.is_null() {
            leaf["expected"] = expected;
        }
        let condition: UniversePrecondition = serde_json::from_value(leaf).unwrap();
        assert!(evaluate_universe_precondition(&current, &condition).unwrap());
    }
    assert_eq!(rows(&state).await, before);
    let cleared = edited(&state, json!([
        {"operation":"clear_fact","target":"facts.facts.kettle"},
        {"operation":"clear_fact","target":"facts.affect.energy"},
        {"operation":"clear_numeric","target":"numeric_values.numeric_values.jars"},
        {"operation":"remove_membership","target":"set_memberships.set_memberships.toolbox","payload":"invented-spanner"}
    ])).await;
    assert_eq!(cleared["facts"], json!({}));
    assert_eq!(cleared["numeric_values"], json!({}));
    assert!(cleared["set_memberships"].get("set_memberships.toolbox").is_none());
    assert_eq!(
        cleared["event_markers"],
        read(&state).await["event_markers"]
    );
}

#[tokio::test]
async fn a_task_blocked_by_a_precondition_is_planned_once_the_route_records_the_fact() {
    let state = state().await;
    let waiting = admit_task(
        &state,
        "Synthetic: make the tea",
        json!({"preconditions": {"target": "facts.kettle.descaled", "predicate": "equals", "expected": true}}),
    )
    .await;
    admit_task(&state, "Synthetic: rinse the cup", json!({})).await;
    assert_eq!(blocked_task_ids(&state).await, vec![waiting.clone()]);

    edited(
        &state,
        json!([{"operation":"set_fact","target":"facts.kettle.descaled","payload":true}]),
    )
    .await;
    assert!(blocked_task_ids(&state).await.is_empty());

    edited(
        &state,
        json!([{"operation":"clear_fact","target":"facts.kettle.descaled"}]),
    )
    .await;
    assert_eq!(blocked_task_ids(&state).await, vec![waiting]);
}

#[tokio::test]
async fn the_document_names_the_route_and_its_five_schemas() {
    let state = state().await;
    let (status, document) = request(&state, "GET", "/openapi.json", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let path = &document["paths"]["/universe-state"];
    assert!(path["get"].is_object() && path["patch"].is_object(), "{path}");
    let committed: Value = serde_json::from_str(include_str!("../openapi/openapi.generated.json")).unwrap();
    assert_eq!(committed["paths"]["/universe-state"], *path);
    // The committed document is the served one, schema for schema: a route whose
    // bodies changed and whose document did not would differ here.
    for schema in [
        "UniverseMutationBody",
        "UniverseStateEditRequest",
        "UniverseStateResponse",
        "ProvenanceKindBody",
        "FactProvenanceBody",
    ] {
        let served = &document["components"]["schemas"][schema];
        assert!(served.is_object(), "{schema}");
        assert_eq!(committed["components"]["schemas"][schema], *served, "{schema}");
    }
    assert_eq!(
        document["components"]["schemas"]["ProvenanceKindBody"]["enum"],
        json!(["asserted", "measured", "derived", "proposed"])
    );
    assert!(document["components"]["schemas"]["UniverseMutationBody"]["properties"]["note"].is_null());
    assert_eq!(committed["paths"].as_object().unwrap().len(), 56);
}

// ---- P1B-59: a measured number is a first-class fact.

#[tokio::test]
async fn a_number_is_set_outright_and_cleared_outright() {
    let state = state().await;
    let litres = "numeric_values.shelf.litres";
    let first = edited(&state, json!([{"operation":"set_numeric","target":litres,"payload":0.7}])).await;
    assert_eq!(first["numeric_values"], json!({"shelf.litres": 0.7}));

    // The case P1B-58's screen could not do. It sent the difference, 0.7 - 0.1,
    // as a decrement, and the number landed on 0.09999999999999998.
    let second = edited(&state, json!([{"operation":"set_numeric","target":litres,"payload":0.1}])).await;
    assert_eq!(second["numeric_values"]["shelf.litres"].as_f64(), Some(0.1));
    assert_eq!(read(&state).await["numeric_values"]["shelf.litres"].as_f64(), Some(0.1));
    assert_ne!(0.7_f64 - (0.7 - 0.1), 0.1, "the difference would not have landed");

    // And a number can be removed, which no operation did before.
    let cleared = edited(&state, json!([{"operation":"clear_numeric","target":litres}])).await;
    assert_eq!(cleared["numeric_values"], json!({}));
    assert_eq!(cleared["fact_provenance"], json!({}));
    // Clearing what is not there is not an error.
    let again = edited(&state, json!([{"operation":"clear_numeric","target":litres}])).await;
    assert_eq!(again["numeric_values"], json!({}));
}

#[tokio::test]
async fn a_write_records_how_the_fact_was_established_and_a_clear_removes_the_record() {
    let state = state().await;
    let written = edited(
        &state,
        json!([
            {"operation":"set_numeric","target":"numeric_values.tank.level","payload":25,"provenance_kind":"measured"},
            {"operation":"set_fact","target":"facts.kettle.descaled","payload":true},
            {"operation":"add_membership","target":"set_memberships.toolbox","payload":"spanner","provenance_kind":"proposed"},
            {"operation":"increment_numeric","target":"numeric_values.shelf.jars","payload":2,"provenance_kind":"derived"}
        ]),
    )
    .await;
    // The write time is this orchestrator's clock, which the test fixes.
    let entry = |kind: &str| json!({"kind": kind, "recorded_at": NOW});
    assert_eq!(
        written["fact_provenance"],
        json!({
            "numeric_values.tank.level": entry("measured"),
            "facts.kettle.descaled": entry("asserted"),
            "set_memberships.toolbox": entry("proposed"),
            "numeric_values.shelf.jars": entry("derived")
        })
    );
    // It is stored, beside the envelope and not in place of it.
    let stored: Value = serde_json::from_str(&rows(&state).await[0].3).unwrap();
    assert_eq!(stored["fact_provenance"]["numeric_values.tank.level"], entry("measured"));
    assert_eq!(stored["provenance"]["authority_source"], "user");
    assert_eq!(read(&state).await, written);

    // A value written again on someone's word is asserted again.
    let reworded = edited(&state, json!([{"operation":"set_numeric","target":"numeric_values.tank.level","payload":18}])).await;
    assert_eq!(reworded["fact_provenance"]["numeric_values.tank.level"], entry("asserted"));

    // Each removal takes the record with the value. None is left for a value that is gone.
    let removed = edited(
        &state,
        json!([
            {"operation":"clear_numeric","target":"numeric_values.tank.level"},
            {"operation":"clear_fact","target":"facts.kettle.descaled"},
            {"operation":"remove_membership","target":"set_memberships.toolbox","payload":"spanner"}
        ]),
    )
    .await;
    assert_eq!(removed["fact_provenance"], json!({"numeric_values.shelf.jars": entry("derived")}));
    for target in removed["fact_provenance"].as_object().unwrap().keys() {
        let (collection, key) = target.split_once('.').unwrap();
        assert!(removed[collection].get(key).is_some(), "{target} has provenance and no value");
    }
}

#[tokio::test]
async fn a_state_from_before_provenance_reads_with_an_empty_map() {
    let state = state().await;
    admit_state(&state, json!({"lamp.bulb": "fitted"})).await;
    let body = read(&state).await;
    assert_eq!(body["facts"], json!({"lamp.bulb": "fitted"}));
    assert_eq!(body["fact_provenance"], json!({}));
    // On an empty store too: the key is always there.
    assert_eq!(read(&crate::state().await).await["fact_provenance"], json!({}));
}

#[tokio::test]
async fn a_completed_task_records_the_provenance_its_effect_states() {
    let state = state().await;
    let task = admit_task(
        &state,
        "Synthetic: read the invented gauge",
        json!({"effects": {"mutations": [
            {"operation":"set_numeric","target":"numeric_values.tank.level","payload":31.5,"provenance_kind":"measured"},
            {"operation":"set_fact","target":"facts.tank.checked","payload":true}
        ]}}),
    )
    .await;
    let (status, body) = request(
        &state,
        "POST",
        &format!("/task/{task}/action"),
        json!({"schema_version": "ubu.orchestrator.task_action.v1", "action": "complete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["diagnostics"], json!([]), "{body}");

    let after = read(&state).await;
    assert_eq!(after["numeric_values"], json!({"tank.level": 31.5}));
    assert_eq!(after["fact_provenance"]["numeric_values.tank.level"]["kind"], "measured");
    assert_eq!(after["fact_provenance"]["facts.tank.checked"]["kind"], "asserted");
    // One completion, one write time, for both.
    assert_eq!(
        after["fact_provenance"]["numeric_values.tank.level"]["recorded_at"],
        after["fact_provenance"]["facts.tank.checked"]["recorded_at"]
    );
}

#[tokio::test]
async fn a_task_waiting_on_a_number_is_planned_once_the_number_is_at_least_what_it_asks() {
    // The planner's precondition path needs no change for the four comparisons:
    // it calls the evaluator, and the evaluator has them. This is that, over HTTP.
    let state = state().await;
    let level = "numeric_values.tank.level";
    let (status, captured) = request(
        &state,
        "POST",
        "/task",
        json!({
            "schema_version": "ubu.orchestrator.task_capture.v1",
            "title": "Synthetic: water the invented bench",
            "duration_estimate": {"type": "fixed", "seconds": 600},
            "preconditions": {"target": level, "predicate": "at_least", "expected": 25}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{captured}");
    let waiting = captured["task_id"].as_str().unwrap().to_owned();
    admit_task(&state, "Synthetic: rinse the cup", json!({})).await;

    // Never recorded: not ready, and not an error.
    assert_eq!(blocked_task_ids(&state).await, vec![waiting.clone()]);
    for (value, blocked) in [(24.5, true), (25.0, false), (40.0, false), (10.0, true)] {
        edited(&state, json!([{"operation":"set_numeric","target":level,"payload":value,"provenance_kind":"measured"}])).await;
        let expected = if blocked { vec![waiting.clone()] } else { Vec::new() };
        assert_eq!(blocked_task_ids(&state).await, expected, "tank.level {value} at_least 25");
    }
    // Cleared, it is never-recorded again.
    edited(&state, json!([{"operation":"set_numeric","target":level,"payload":40}])).await;
    assert!(blocked_task_ids(&state).await.is_empty());
    edited(&state, json!([{"operation":"clear_numeric","target":level}])).await;
    assert_eq!(blocked_task_ids(&state).await, vec![waiting.clone()]);

    // A comparison that cannot be evaluated is an invalid Task, not a blocked one.
    let (status, odd) = request(
        &state,
        "POST",
        "/task",
        json!({
            "schema_version": "ubu.orchestrator.task_capture.v1",
            "title": "Synthetic: count the invented jars",
            "duration_estimate": {"type": "fixed", "seconds": 600},
            "preconditions": {"target": "facts.shelf.jars", "predicate": "greater_than", "expected": 3}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{odd}");
    let (status, planned) = request(&state, "POST", "/planning/generate", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{planned}");
    assert_eq!(planned["invalid_tasks"][0]["task_id"], odd["task_id"]);
    assert_eq!(
        planned["invalid_tasks"][0]["error"],
        "malformed precondition: greater_than requires a numeric_values target"
    );
}

#[tokio::test]
async fn a_task_route_refuses_a_note_on_a_mutation() {
    // The mutation type has no `note`, wherever a mutation is written.
    let state = state().await;
    let (status, body) = request(
        &state,
        "POST",
        "/task",
        json!({
            "schema_version": "ubu.orchestrator.task_capture.v1",
            "title": "Synthetic: an effect with a note",
            "effects": {"mutations": [{"operation":"set_fact","target":"facts.kettle.descaled","payload":true,"note":"nowhere to go"}]}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
