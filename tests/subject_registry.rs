//! Invented roots only; in-process routes, no external transport.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::request;
use serde_json::json;
use ubu_orchestrator::{
    config::ServerConfig,
    services::{setting_authoring, subject_vocabulary},
    state::AppState,
};
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env()).await.unwrap()
}
#[tokio::test]
async fn explicit_mint_is_a_setting_and_referenced_retirement_is_refused_without_cascading() {
    let state = state().await;
    let before = subject_vocabulary::effective(&state).await.unwrap();
    assert_eq!(
        before,
        subject_vocabulary::GOVERNED
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
    let (status, _) = request(
        &state,
        "PUT",
        "/setting/universe.subject.teapot",
        json!({"schema_version":"ubu.orchestrator.setting.v1","value":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let roots = subject_vocabulary::effective(&state).await.unwrap();
    assert!(roots.contains("teapot"));
    assert_eq!(roots.len(), 6);
    let listed = setting_authoring::list(&state).await.unwrap();
    assert_eq!(listed.settings.len(), 1);
    assert_eq!(listed.settings[0].name, "universe.subject.teapot");
    assert_eq!(listed.settings[0].value, true);
    assert_eq!(listed.settings[0].authority_source, "user");
    let (status, world) = request(&state, "PATCH", "/universe-state", json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_fact","target":"facts.teapot.ready","payload":true}]})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, refusal) = request(
        &state,
        "DELETE",
        "/setting/universe.subject.teapot",
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(refusal["diagnostics"][0]["code"], "subject_referenced");
    assert!(subject_vocabulary::effective(&state).await.unwrap().contains("teapot"));
    let (_, after) = request(&state, "GET", "/universe-state", json!(null)).await;
    assert_eq!(after, world);
    let (status, _) = request(&state, "PATCH", "/universe-state", json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"clear_fact","target":"facts.teapot.ready"}]})).await;
    assert_eq!(status, StatusCode::OK);
    setting_authoring::delete(&state, "universe.subject.teapot").await.unwrap();
    assert_eq!(subject_vocabulary::effective(&state).await.unwrap(), before);
}
#[tokio::test]
async fn reserved_and_governed_roots_cannot_be_minted_or_retired() {
    let state = state().await;
    for root in [
        "facts",
        "numeric_values",
        "set_memberships",
        "event_markers",
        "affect",
        "operator",
        "project",
        "github",
        "relationship",
    ] {
        for method in ["PUT", "DELETE"] {
            let (status, body) = request(
                &state,
                method,
                &format!("/setting/universe.subject.{root}"),
                json!({"schema_version":"ubu.orchestrator.setting.v1","value":true}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(body["diagnostics"][0]["code"]
                .as_str()
                .unwrap()
                .starts_with("subject_"));
        }
    }
    assert!(setting_authoring::list(&state)
        .await
        .unwrap()
        .settings
        .is_empty());
}
#[tokio::test]
async fn malformed_duplicate_and_non_true_mints_leave_the_registry_unchanged() {
    let state = state().await;
    for root in [
        "",
        "Teapot",
        "teapot.room",
        "tea-pot",
        "tea__pot",
        "1teapot",
        &"a".repeat(65),
    ] {
        assert!(
            setting_authoring::put(&state, &format!("universe.subject.{root}"), json!(true))
                .await
                .is_err()
        );
    }
    for value in [json!(false), json!("true"), json!(0), json!(null)] {
        assert!(
            setting_authoring::put(&state, "universe.subject.teapot", value)
                .await
                .is_err()
        );
    }
    setting_authoring::put(&state, "universe.subject.teapot", json!(true))
        .await
        .unwrap();
    assert!(
        setting_authoring::put(&state, "universe.subject.teapot", json!(true))
            .await
            .is_err()
    );
    assert_eq!(
        setting_authoring::list(&state)
            .await
            .unwrap()
            .settings
            .len(),
        1
    );
    assert!(
        setting_authoring::delete(&state, "universe.subject.workbench")
            .await
            .is_err()
    );
}
