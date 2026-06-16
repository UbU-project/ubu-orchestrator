use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use ubu_orchestrator::api::projection::{
    PROJECTION_APPROVAL_SCHEMA_VERSION, PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION,
    PROJECTION_PREVIEW_SCHEMA_VERSION, PROJECTION_RECONCILIATION_SCHEMA_VERSION,
};
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::test]
async fn projection_preview_is_deterministic_label_only_and_schema_checked() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state);

    let missing_schema = app
        .clone()
        .oneshot(json_request("/projection/preview", "{}"))
        .await
        .expect("missing schema response");
    assert_eq!(missing_schema.status(), StatusCode::BAD_REQUEST);
    let body = response_json(missing_schema).await;
    assert_eq!(body["diagnostics"][0]["code"], "missing_schema_version");

    let request = format!(
        r#"{{
            "schema_version":"{PROJECTION_PREVIEW_SCHEMA_VERSION}",
            "owner":"UbU-project",
            "repo":"ubu-orchestrator",
            "issue_number":7,
            "observed_labels":[],
            "desired_labels":["ubu-managed"],
            "existing_repository_labels":["ubu","ubu-managed"]
        }}"#
    );
    let first = response_json(
        app.clone()
            .oneshot(json_request("/projection/preview", &request))
            .await
            .expect("first preview"),
    )
    .await;
    let second = response_json(
        app.oneshot(json_request("/projection/preview", &request))
            .await
            .expect("second preview"),
    )
    .await;

    assert_eq!(first["schema_version"], PROJECTION_PREVIEW_SCHEMA_VERSION);
    assert_eq!(first["operations"][0]["kind"], "label");
    assert_eq!(
        first["operations"][0]["operation_id"],
        second["operations"][0]["operation_id"]
    );
    assert_eq!(first["policy_summary"]["legitimization"], "accepted");
    assert!(first["requires_approval"].as_bool().unwrap());
}

#[tokio::test]
async fn rejected_projection_is_logged_and_not_written() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let pool = state.inner().store.pool().clone();
    let app = ubu_orchestrator::build_router(state);

    let preview = create_preview(&app, true).await;
    let preview_id = preview["preview_id"].as_str().expect("preview id");
    let approve = format!(
        r#"{{
            "schema_version":"{PROJECTION_APPROVAL_SCHEMA_VERSION}",
            "preview_id":"{preview_id}",
            "approved":true,
            "authority_source":"user"
        }}"#
    );
    let result = response_json(
        app.oneshot(json_request("/projection/approve", &approve))
            .await
            .expect("approval"),
    )
    .await;

    assert_eq!(result["status"], "failed");
    assert_eq!(result["diagnostics"][0]["code"], "projection_denied");
    assert_eq!(worker_write_count(&pool).await, 0);
    assert_eq!(boundary_log_count(&pool).await, 1);
}

#[tokio::test]
async fn reconciliation_surfaces_conflict_and_accepts_external_change() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state);

    let preview = create_preview(&app, false).await;
    let preview_id = preview["preview_id"].as_str().expect("preview id");
    let approve = format!(
        r#"{{
            "schema_version":"{PROJECTION_APPROVAL_SCHEMA_VERSION}",
            "preview_id":"{preview_id}",
            "approved":true,
            "authority_source":"user"
        }}"#
    );
    let result = response_json(
        app.clone()
            .oneshot(json_request("/projection/approve", &approve))
            .await
            .expect("approval"),
    )
    .await;
    assert_eq!(result["status"], "applied");
    assert_eq!(
        result["operation_results"][0]["authority_source"],
        "automation_worker"
    );

    let reconcile = format!(
        r#"{{
            "schema_version":"{PROJECTION_RECONCILIATION_SCHEMA_VERSION}",
            "observed_labels":[]
        }}"#
    );
    let reconciliation = response_json(
        app.clone()
            .oneshot(json_request("/projection/reconcile", &reconcile))
            .await
            .expect("reconcile"),
    )
    .await;
    assert_eq!(reconciliation["status"], "missing");
    assert_eq!(
        reconciliation["diagnostics"][0]["code"],
        "projection_conflict"
    );

    let reconciliation_id = reconciliation["reconciliation_id"]
        .as_str()
        .expect("reconciliation id");
    let conflict_operation_id = reconciliation["conflicts"][0]["operation_id"]
        .as_str()
        .expect("conflict operation id");
    let accept = format!(
        r#"{{
            "schema_version":"{PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION}",
            "reconciliation_id":"{reconciliation_id}",
            "conflict_operation_id":"{conflict_operation_id}",
            "authority_source":"user"
        }}"#
    );
    let accepted = response_json(
        app.oneshot(json_request(
            "/projection/reconciliation/accept-external",
            &accept,
        ))
        .await
        .expect("accept external"),
    )
    .await;
    assert!(accepted["admitted_object_id"]
        .as_str()
        .expect("admitted id")
        .starts_with("xevent_"));
}

async fn create_preview(app: &axum::Router, no_external_export: bool) -> Value {
    let request = format!(
        r#"{{
            "schema_version":"{PROJECTION_PREVIEW_SCHEMA_VERSION}",
            "owner":"UbU-project",
            "repo":"ubu-orchestrator",
            "issue_number":7,
            "observed_labels":[],
            "desired_labels":["ubu-managed"],
            "existing_repository_labels":["ubu","ubu-managed"],
            "no_external_export":{no_external_export}
        }}"#
    );
    let response = app
        .clone()
        .oneshot(json_request("/projection/preview", &request))
        .await
        .expect("preview response");
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn boundary_log_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM logs WHERE event_type = 'compartment_boundary_decided'",
    )
    .fetch_one(pool)
    .await
    .expect("log count")
}

async fn worker_write_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM projection_worker_writes")
        .fetch_one(pool)
        .await
        .expect("worker write count")
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

fn json_request(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request")
}
