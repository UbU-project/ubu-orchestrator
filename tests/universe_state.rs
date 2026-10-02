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
async fn each_of_the_seven_operations_round_trips() {
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
    // One row, eight versions: the seed and one per edit.
    let rows = rows(&state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, 8);
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
        json!({"schema_version": SCHEMA, "mutations": [{"operation":"set_fact"}]}),
        json!({"schema_version": SCHEMA}),
    ] {
        let (status, _) = request(&state, "PATCH", "/universe-state", body.clone()).await;
        assert!(status.is_client_error(), "{body}: {status}");
    }
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn an_intrinsic_affect_target_is_the_operators_to_set_in_user_mode() {
    // This instance runs in `user_mode`, which models intrinsic affect. The refusal
    // in the other two modes is asserted beside the service, which takes the mode.
    let state = state().await;
    let written = edited(
        &state,
        json!([{"operation":"increment_numeric","target":"numeric_values.affect.energy","payload":1.5}]),
    )
    .await;
    assert_eq!(written["numeric_values"], json!({"affect.energy": 1.5}));
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
async fn the_document_names_the_route_and_its_three_schemas() {
    let state = state().await;
    let (status, document) = request(&state, "GET", "/openapi.json", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let path = &document["paths"]["/universe-state"];
    assert!(path["get"].is_object() && path["patch"].is_object(), "{path}");
    for schema in ["UniverseMutationBody", "UniverseStateEditRequest", "UniverseStateResponse"] {
        assert!(document["components"]["schemas"][schema].is_object(), "{schema}");
    }
    let committed: Value = serde_json::from_str(include_str!("../openapi/openapi.generated.json")).unwrap();
    assert_eq!(committed["paths"]["/universe-state"], *path);
}
