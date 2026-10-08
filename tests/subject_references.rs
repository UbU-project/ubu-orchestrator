//! Synthetic in-process stores/routes only; no transport or process effects.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::request;
use serde_json::{json, Value};
use ubu_core::{core::UniverseState, AuthoritySource, VersionRef};
use ubu_orchestrator::{config::ServerConfig, services::{setting_authoring, subject_vocabulary}, state::AppState};
use ubu_store::{models::object_record::NewObjectRecord, queries};

async fn state() -> AppState { AppState::in_memory(ServerConfig::from_env()).await.unwrap() }
async fn mint(state: &AppState, root: &str) {
    setting_authoring::put(state, &format!("universe.subject.{root}"), json!(true)).await.unwrap();
}
async fn edit(state: &AppState, mutations: Value) -> Value {
    let (status, body) = request(state, "PATCH", "/universe-state", json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":mutations})).await;
    assert_eq!(status, 200, "{body}"); body
}
async fn stored_world(state: &AppState, world: UniverseState, updated: &str) {
    let now = world.captured_at;
    let envelope = state.envelope_for([(world.id.clone(), VersionRef::Absent)].into_iter().collect(), AuthoritySource::User, now).unwrap();
    let mut payload = serde_json::to_value(&world).unwrap();
    payload["schema_version"] = json!("core/universe-state/0.1");
    payload["provenance"] = json!({"created_at":now,"authority_source":"user"});
    queries::admit_object(state.inner().store.pool(), &envelope, NewObjectRecord {
        id:world.id.to_string(), object_type:"UniverseState".into(), version:1, status:"active".into(),
        compartment_label:"user-capture".into(), payload, created_at:now.to_string(), updated_at:updated.into(),
    }).await.unwrap();
}

#[tokio::test]
async fn registry_metadata_counts_all_three_spaces_without_values_or_substring_matches() {
    let state = state().await;
    for root in ["shelf", "shelf_rack", "kettle"] { mint(&state, root).await; }
    edit(&state, json!([
        {"operation":"set_fact","target":"facts.shelf.ready","payload":{"target":"facts.kettle.decoy"}},
        {"operation":"set_numeric","target":"numeric_values.shelf.height","payload":4},
        {"operation":"add_membership","target":"set_memberships.shelf.parts","payload":"facts.kettle.decoy"},
        {"operation":"append_event_marker","target":"event_markers.shelf.checked","payload":{"target":"facts.kettle.decoy"}},
        {"operation":"set_fact","target":"facts.shelf_rack.ready","payload":true}
    ])).await;
    // A non-latest object row still carries a reference and cannot be ignored.
    let mut older = UniverseState::new(state.planning_now(), "Synthetic older shelf state");
    older.facts.insert("shelf.old_ready".into(), json!(true));
    stored_world(&state, older, "2020-01-01T00:00:00Z").await;
    fixture::seed(&state, fixture::A, "completed", json!({"description":"facts.kettle.decoy", "effects":{"mutations":[{"operation":"set_fact","target":"facts.kettle.decoy","payload":true}]}, "preconditions":{"all_of":[
        {"target":"facts.shelf.ready","predicate":"equals","expected":{"target":"facts.kettle.decoy"}},
        {"any_of":[{"target":"numeric_values.shelf.height","predicate":"at_least","expected":1},{"target":"facts.shelf.ready","predicate":"absent"}]}
    ]}})).await;
    let (_, listed) = request(&state, "GET", "/settings", json!(null)).await;
    let shelf = listed["settings"].as_array().unwrap().iter().find(|r| r["name"] == "universe.subject.shelf").unwrap();
    assert_eq!(shelf["version"], 1);
    let row = setting_authoring::current(&state, "universe.subject.shelf").await.unwrap().unwrap();
    assert_eq!(shelf["subject_metadata"]["minted_at"], row.created_at);
    assert_eq!(shelf["subject_metadata"]["references"], json!({"universe_state_keys":5,"fact_provenance_keys":4,"task_precondition_targets":3}));
    assert!(!shelf.to_string().contains("old_ready"));
    let counts = subject_vocabulary::reference_counts(&state).await.unwrap();
    assert!(!counts.contains_key("kettle"));
    assert_eq!(counts["shelf_rack"].universe_state_keys, 1);
    let before = fixture::canonical_rows(&state).await;
    let (status, refusal) = request(&state, "DELETE", "/setting/universe.subject.shelf", json!(null)).await;
    assert_eq!(status, 409);
    assert_eq!(refusal["diagnostics"][0]["code"], "subject_referenced");
    let message = refusal["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains("UniverseState keys 5; fact_provenance keys 4; Task precondition targets 3"));
    for private in ["shelf.ready", "old_ready", "decoy", "universe.subject.shelf"] { assert!(!message.contains(private)); }
    assert_eq!(fixture::canonical_rows(&state).await, before);
}

#[tokio::test]
async fn task_only_references_refuse_retirement_until_explicit_requirement_clear() {
    let state = state().await; mint(&state, "kettle").await;
    fixture::seed(&state, fixture::A, "active", json!({"preconditions":{"any_of":[{"target":"facts.kettle.ready","predicate":"absent"}]}})).await;
    let (status, refusal) = request(&state, "DELETE", "/setting/universe.subject.kettle", json!(null)).await;
    assert_eq!(status, 409);
    assert!(refusal["error"].as_str().unwrap().contains("UniverseState keys 0; fact_provenance keys 0; Task precondition targets 1"));
    let (status, _) = request(&state, "PATCH", &format!("/task/{}", fixture::A), json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":null})).await;
    assert_eq!(status, 200);
    let (status, _) = request(&state, "DELETE", "/setting/universe.subject.kettle", json!(null)).await;
    assert_eq!(status, 204);
}

#[tokio::test]
async fn provenance_only_reference_is_not_discarded_as_an_orphan() {
    let state = state().await; mint(&state, "kettle").await;
    let mut world = UniverseState::new(state.planning_now(), "Synthetic provenance-only state");
    world.fact_provenance.insert("facts.kettle.old_ready".into(), ubu_core::core::FactProvenance { kind:ubu_core::core::ProvenanceKind::Asserted, recorded_at:state.planning_now() });
    stored_world(&state, world, "2020-01-01T00:00:00Z").await;
    let (status, body) = request(&state, "DELETE", "/setting/universe.subject.kettle", json!(null)).await;
    assert_eq!(status, 409);
    assert!(body["error"].as_str().unwrap().contains("UniverseState keys 0; fact_provenance keys 1; Task precondition targets 0"));
}

#[tokio::test]
async fn append_only_markers_hold_the_root_and_retirement_does_not_cascade() {
    let state = state().await; mint(&state, "kettle").await;
    let world = edit(&state, json!([{"operation":"append_event_marker","target":"event_markers.kettle.boiled","payload":{"synthetic":true}}])).await;
    let (status, refusal) = request(&state, "DELETE", "/setting/universe.subject.kettle", json!(null)).await;
    assert_eq!(status, 409);
    assert!(refusal["error"].as_str().unwrap().contains("Append-only event markers have no clearing operation"));
    assert_eq!(request(&state, "GET", "/universe-state", json!(null)).await.1, world);
    assert!(subject_vocabulary::effective(&state).await.unwrap().contains("kettle"));
}

#[tokio::test]
async fn malformed_count_input_fails_closed_without_exposing_stored_content() {
    let state = state().await; mint(&state, "kettle").await;
    let world = edit(&state, json!([{"operation":"set_fact","target":"facts.kettle.ready","payload":true}])).await;
    sqlx::query("UPDATE objects SET payload_json=json_set(payload_json,'$.facts',json(?)) WHERE id=?")
        .bind("[\"synthetic_private_count_canary\"]").bind(world["id"].as_str().unwrap()).execute(state.inner().store.pool()).await.unwrap();
    let (status, refusal) = request(&state, "DELETE", "/setting/universe.subject.kettle", json!(null)).await;
    assert_eq!(status, 500);
    assert!(!refusal.to_string().contains("synthetic_private_count_canary"));
    assert!(setting_authoring::current(&state, "universe.subject.kettle").await.unwrap().is_some());
}
