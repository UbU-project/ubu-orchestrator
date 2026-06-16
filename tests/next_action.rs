use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_orchestrator::api::next_action::NEXT_ACTION_SCHEMA_VERSION;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

#[tokio::test]
async fn next_action_selects_ready_task_by_priority_and_explains_it() {
    let state = test_state().await;
    let objective_id = admit_objective(&state, "Ship O6").await;
    let lower_priority_task = admit_task(
        &state,
        "Do later",
        Some(&objective_id),
        Some(20),
        Vec::new(),
        true,
        "issue-20",
    )
    .await;
    let selected_task = admit_task(
        &state,
        "Do first",
        Some(&objective_id),
        Some(10),
        Vec::new(),
        true,
        "issue-10",
    )
    .await;

    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(get_request(&format!(
            "/next-action?schema_version={NEXT_ACTION_SCHEMA_VERSION}"
        )))
        .await
        .expect("next action response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["schema_version"], NEXT_ACTION_SCHEMA_VERSION);
    assert_eq!(body["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(body["recommendation"]["task_id"], selected_task);
    assert_eq!(body["recommendation"]["title"], "Do first");
    assert_eq!(body["recommendation"]["readiness"], "ready");
    assert_eq!(
        body["recommendation"]["parent_objective"]["objective_id"],
        objective_id
    );
    assert_eq!(
        body["recommendation"]["source_refs"][0]["source_id"],
        "issue-10"
    );
    assert_eq!(
        body["recommendation"]["explanation"]["label"],
        "readiness-based recommendation"
    );
    assert!(body["recommendation"]["explanation"]["message"]
        .as_str()
        .unwrap()
        .contains("parent Objective 'Ship O6'"));

    assert_ne!(lower_priority_task, selected_task);
}

#[tokio::test]
async fn next_action_returns_bounded_diagnostic_when_all_tasks_are_blocked() {
    let state = test_state().await;
    let dependency_id = admit_task(
        &state,
        "Dependency",
        None,
        Some(1),
        Vec::new(),
        true,
        "issue-dependency",
    )
    .await;
    admit_task(
        &state,
        "Blocked task",
        None,
        Some(1),
        vec![dependency_id.clone()],
        true,
        "issue-blocked",
    )
    .await;

    let pool = state.inner().store.pool();
    sqlx::query("UPDATE objects SET status = 'failed' WHERE id = ?")
        .bind(&dependency_id)
        .execute(pool)
        .await
        .expect("dependency marked non-active");

    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(get_request(&format!(
            "/next-action?schema_version={NEXT_ACTION_SCHEMA_VERSION}"
        )))
        .await
        .expect("next action response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert!(body["recommendation"].is_null());
    assert_eq!(
        body["diagnostics"][0]["code"],
        "all_candidates_blocked_on_unmet_dependencies"
    );
    assert_eq!(body["diagnostics"][0]["blocked_task_count"], 1);
    assert_eq!(
        body["diagnostics"][0]["sampled_task_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn next_action_requires_known_schema_version() {
    let state = test_state().await;
    let app = ubu_orchestrator::build_router(state);

    let response = app
        .oneshot(get_request("/next-action"))
        .await
        .expect("next action response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "missing_schema_version");
}

async fn test_state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state")
}

async fn admit_objective(state: &AppState, title: &str) -> String {
    let id = UbuId::new(ObjectType::Objective).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    queries::admit_object(
        state.inner().store.pool(),
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Objective.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload: json!({
                "id": id.clone(),
                "title": title,
                "status": "active",
                "provenance": provenance(&now, "test", "objective")
            }),
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("objective admitted");
    id
}

async fn admit_task(
    state: &AppState,
    title: &str,
    objective_id: Option<&str>,
    priority: Option<i64>,
    blocked_by: Vec<String>,
    precondition_satisfied: bool,
    source_id: &str,
) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let mut payload = json!({
        "id": id.clone(),
        "title": title,
        "status": "active",
        "priority": priority,
        "blocked_by": blocked_by,
        "precondition_satisfied": precondition_satisfied,
        "provenance": provenance(&now, "github_issue", source_id)
    });
    if let Some(objective_id) = objective_id {
        payload["objective_id"] = json!(objective_id);
    }

    queries::admit_object(
        state.inner().store.pool(),
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("task admitted");
    id
}

fn provenance(created_at: &str, source_kind: &str, source_id: &str) -> Value {
    json!({
        "created_at": created_at,
        "authority_source": "user",
        "source_refs": [{
            "source_kind": source_kind,
            "source_id": source_id,
            "url": format!("https://example.test/{source_id}")
        }]
    })
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}
