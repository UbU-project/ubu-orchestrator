//! New-write governance without rewriting invented legacy state.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::request;
use serde_json::{json, Value};
use ubu_core::core::{evaluate_universe_precondition, InstanceMode, UniverseMutation};
use ubu_orchestrator::{
    config::ServerConfig,
    services::{precondition_advisor, setting_authoring, subject_vocabulary, universe_state},
    state::AppState,
};
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env()).await.unwrap()
}
async fn edit(state: &AppState, mutations: Value) -> (axum::http::StatusCode, Value) {
    request(
        state,
        "PATCH",
        "/universe-state",
        json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":mutations}),
    )
    .await
}
#[tokio::test]
async fn governed_provisional_and_entity_path_writes_succeed_in_all_collections() {
    let state = state().await;
    setting_authoring::put(&state, "universe.subject.teapot", json!(true))
        .await
        .unwrap();
    let (status, body) = edit(&state,json!([
        {"operation":"set_fact","target":"facts.operator.work_style","payload":"synthetic"},
        {"operation":"set_fact","target":"facts.github.issue.14.pipeline_state","payload":"synthetic"},
        {"operation":"set_numeric","target":"numeric_values.teapot.charge","payload":0,"provenance_kind":"measured"},
        {"operation":"add_membership","target":"set_memberships.project.tools","payload":"synthetic spanner"},
        {"operation":"append_event_marker","target":"event_markers.teapot.boiled","payload":{"synthetic":true}}
    ])).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["facts"]["operator.work_style"], "synthetic");
    assert_eq!(body["numeric_values"]["teapot.charge"].as_f64(), Some(0.0));
    assert_eq!(
        body["fact_provenance"]["numeric_values.teapot.charge"]["kind"],
        "measured"
    );
}
#[tokio::test]
async fn unknown_single_segment_and_bad_predicate_refuse_whole_without_a_seed() {
    let state = state().await;
    for (target, code) in [
        ("facts.teapot.ready", "universe_target_subject_unknown"),
        ("facts.single_leaf", "universe_target_grammar_invalid"),
        (
            "facts.operator.Upper_leaf",
            "universe_target_grammar_invalid",
        ),
        ("facts.operator.issue..ready", "universe_mutation_invalid"),
    ] {
        let (status, body) = edit(&state,json!([{"operation":"set_fact","target":"facts.operator.ready","payload":true},{"operation":"set_fact","target":target,"payload":true}])).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["diagnostics"][0]["code"], code);
        if code == "universe_target_subject_unknown" {
            assert_eq!(body["diagnostics"][0]["message"],"Subject `teapot` is not in the effective vocabulary. Mint it explicitly in UniverseState's Subjects list before writing `facts.teapot.ready`.");
        }
        assert_eq!(universe_state::read(&state).await.unwrap().1, None);
    }
}
#[tokio::test]
async fn a_subject_can_retire_after_references_are_cleared_and_legacy_targets_still_work() {
    let state = state().await;
    setting_authoring::put(&state, "universe.subject.teapot", json!(true))
        .await
        .unwrap();
    assert_eq!(
        edit(
            &state,
            json!([{"operation":"set_fact","target":"facts.teapot.ready","payload":true}])
        )
        .await
        .0,
        200
    );
    assert!(setting_authoring::delete(&state, "universe.subject.teapot").await.is_err());
    let (world, _) = universe_state::read(&state).await.unwrap();
    assert!(precondition_advisor::targets(&world).contains("facts.teapot.ready"));
    assert!(evaluate_universe_precondition(
        &world,
        &serde_json::from_value(
            json!({"target":"facts.teapot.ready","predicate":"equals","expected":true})
        )
        .unwrap()
    )
    .unwrap());
    assert_eq!(
        edit(
            &state,
            json!([{"operation":"clear_fact","target":"facts.teapot.ready"}])
        )
        .await
        .0,
        200
    );
    setting_authoring::delete(&state, "universe.subject.teapot").await.unwrap();
    assert_eq!(edit(&state, json!([{"operation":"set_fact","target":"facts.teapot.ready","payload":false}])).await.0, 400);
}
#[tokio::test]
async fn stored_single_segment_targets_keep_all_destructive_operations_and_precondition_semantics()
{
    let state = state().await;
    // The unchanged Task-effects path is an existing writer for legacy state.
    fixture::seed(&state,fixture::A,"active",json!({"effects":{"mutations":[
        {"operation":"set_fact","target":"facts.legacy_leaf","payload":true},
        {"operation":"set_numeric","target":"numeric_values.legacy_number","payload":3},
        {"operation":"add_membership","target":"set_memberships.legacy_set","payload":"synthetic"}
    ]}})).await;
    let (status, body) = request(
        &state,
        "POST",
        &format!("/task/{}/action", fixture::A),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (world, _) = universe_state::read(&state).await.unwrap();
    assert!(precondition_advisor::targets(&world).contains("facts.legacy_leaf"));
    assert!(evaluate_universe_precondition(
        &world,
        &serde_json::from_value(
            json!({"target":"facts.legacy_leaf","predicate":"equals","expected":true})
        )
        .unwrap()
    )
    .unwrap());
    assert_eq!(edit(&state,json!([
        {"operation":"clear_fact","target":"facts.legacy_leaf"},
        {"operation":"clear_numeric","target":"numeric_values.legacy_number"},
        {"operation":"remove_membership","target":"set_memberships.legacy_set","payload":"synthetic"}
    ])).await.0,200);
    let (after, _) = universe_state::read(&state).await.unwrap();
    assert!(precondition_advisor::targets(&after).is_empty());
}
#[tokio::test]
async fn reserved_reasons_and_intrinsic_affect_mode_checks_still_win() {
    let state = state().await;
    for root in [
        "facts",
        "numeric_values",
        "set_memberships",
        "event_markers",
        "affect",
    ] {
        let (_, body) = edit(
            &state,
            json!([{"operation":"set_fact","target":format!("facts.{root}.ready"),"payload":true}]),
        )
        .await;
        assert_eq!(
            body["diagnostics"][0]["code"],
            "universe_target_namespace_invalid"
        );
    }
    let mutation = UniverseMutation {
        operation: "set_numeric".into(),
        target: "numeric_values.affect.energy".into(),
        payload: Some(json!(1)),
        provenance_kind: None,
    };
    for mode in [InstanceMode::OrganizationMode, InstanceMode::WorkerMode] {
        assert!(
            universe_state::apply(&state, std::slice::from_ref(&mutation), mode)
                .await
                .unwrap_err()
                .to_string()
                .contains("intrinsic")
        );
    }
    assert_eq!(
        subject_vocabulary::effective(&state).await.unwrap().len(),
        5
    );
}
