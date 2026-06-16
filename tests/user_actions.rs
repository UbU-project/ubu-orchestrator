use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_orchestrator::api::user_action::TASK_ACTION_SCHEMA_VERSION;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

#[tokio::test]
async fn complete_records_decision_log_and_transitions_task() {
    let state = test_state().await;
    let task_id = admit_task(&state, "Complete me").await;
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .oneshot(json_request(
            &format!("/task/{task_id}/action"),
            json!({
                "schema_version": TASK_ACTION_SCHEMA_VERSION,
                "action": "complete",
                "note": "done"
            }),
        ))
        .await
        .expect("action response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["schema_version"], TASK_ACTION_SCHEMA_VERSION);
    assert_eq!(body["action"], "complete");
    assert_eq!(body["task_status"], "completed");
    assert_eq!(body["authority_source"], "user");
    assert_eq!(body["transition_applied"], true);

    let row = sqlx::query("SELECT status, payload_json FROM objects WHERE id = ?")
        .bind(&task_id)
        .fetch_one(state.inner().store.pool())
        .await
        .expect("task row");
    let status: String = row.try_get("status").expect("status");
    let payload_json: String = row.try_get("payload_json").expect("payload");
    let payload: Value = serde_json::from_str(&payload_json).expect("payload json");
    assert_eq!(status, "completed");
    assert_eq!(payload["status"], "completed");

    let log = sqlx::query(
        "SELECT event_type, payload_json, provenance_json FROM logs WHERE object_refs_json LIKE ?",
    )
    .bind(format!("%{task_id}%"))
    .fetch_one(state.inner().store.pool())
    .await
    .expect("log row");
    let event_type: String = log.try_get("event_type").expect("event_type");
    let payload_json: String = log.try_get("payload_json").expect("payload");
    let provenance_json: String = log.try_get("provenance_json").expect("provenance");
    let payload: Value = serde_json::from_str(&payload_json).expect("log payload");
    let provenance: Value = serde_json::from_str(&provenance_json).expect("provenance");
    assert_eq!(event_type, "decision_recorded");
    assert_eq!(payload["action"], "complete");
    assert_eq!(provenance["authority_source"], "user");
}

#[tokio::test]
async fn override_records_user_override_without_transition() {
    let state = test_state().await;
    let task_id = admit_task(&state, "Override me").await;
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .oneshot(json_request(
            &format!("/task/{task_id}/action"),
            json!({
                "schema_version": TASK_ACTION_SCHEMA_VERSION,
                "action": "override",
                "note": "not now"
            }),
        ))
        .await
        .expect("action response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["action"], "override");
    assert_eq!(body["task_status"], "active");
    assert_eq!(body["authority_source"], "user_override");
    assert_eq!(body["transition_applied"], false);

    let row = sqlx::query("SELECT status FROM objects WHERE id = ?")
        .bind(&task_id)
        .fetch_one(state.inner().store.pool())
        .await
        .expect("task row");
    let status: String = row.try_get("status").expect("status");
    assert_eq!(status, "active");

    let log = sqlx::query("SELECT provenance_json FROM logs WHERE object_refs_json LIKE ?")
        .bind(format!("%{task_id}%"))
        .fetch_one(state.inner().store.pool())
        .await
        .expect("log row");
    let provenance_json: String = log.try_get("provenance_json").expect("provenance");
    let provenance: Value = serde_json::from_str(&provenance_json).expect("provenance");
    assert_eq!(provenance["authority_source"], "user_override");
}

#[tokio::test]
async fn record_action_requires_known_schema_version() {
    let state = test_state().await;
    let task_id = admit_task(&state, "Version me").await;
    let app = ubu_orchestrator::build_router(state);

    let response = app
        .oneshot(json_request(
            &format!("/task/{task_id}/action"),
            json!({
                "action": "complete"
            }),
        ))
        .await
        .expect("action response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "missing_schema_version");
}

async fn test_state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state")
}

async fn admit_task(state: &AppState, title: &str) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    queries::admit_object(
        state.inner().store.pool(),
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload: json!({
                "id": id.clone(),
                "title": title,
                "status": "active",
                "provenance": {
                    "created_at": now.clone(),
                    "authority_source": "user",
                    "source": {
                        "source_kind": "test",
                        "source_id": title
                    }
                }
            }),
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("task admitted");
    id
}

fn json_request(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
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
