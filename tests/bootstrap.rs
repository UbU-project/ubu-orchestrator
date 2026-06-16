use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use ubu_orchestrator::api::bootstrap::BOOTSTRAP_SCHEMA_VERSION;
use ubu_orchestrator::api::desktop::DESKTOP_SESSION_SCHEMA_VERSION;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::test]
async fn seed_admits_bootstrap_state_and_imports_selected_repo_tasks() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state.clone());

    let token_response = app
        .clone()
        .oneshot(json_request(
            "/desktop/session/github-token",
            json!({
                "schema_version": DESKTOP_SESSION_SCHEMA_VERSION,
                "github_token": "desktop-session-test-value"
            }),
        ))
        .await
        .expect("token response");
    assert_eq!(token_response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(seed_request())
        .await
        .expect("seed response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schema_version"], BOOTSTRAP_SCHEMA_VERSION);
    assert_eq!(body["objective_ids"].as_array().unwrap().len(), 1);
    assert_eq!(body["preference_ids"].as_array().unwrap().len(), 3);
    assert_eq!(body["imported_tasks"]["admitted_to_store"], 1);

    let rows = sqlx::query("SELECT object_type, payload_json FROM objects ORDER BY object_type")
        .fetch_all(state.inner().store.pool())
        .await
        .expect("objects query");

    let mut objective_count = 0;
    let mut preference_count = 0;
    let mut task_count = 0;
    for row in rows {
        let object_type: String = row.try_get("object_type").expect("object_type");
        let payload_json: String = row.try_get("payload_json").expect("payload_json");
        let payload: Value = serde_json::from_str(&payload_json).expect("payload");
        match object_type.as_str() {
            "Objective" => {
                objective_count += 1;
                assert_eq!(payload["provenance"]["authority_source"], "user");
                assert_eq!(payload["provenance"]["source"]["source_kind"], "bootstrap");
            }
            "Preference" => {
                preference_count += 1;
                assert_eq!(payload["authority_source"], "user");
                assert_eq!(payload["provenance"]["authority_source"], "user");
                assert_eq!(payload["provenance"]["source"]["source_kind"], "bootstrap");
            }
            "Task" => {
                task_count += 1;
                assert_eq!(payload["provenance"]["authority_source"], "system");
                assert_eq!(
                    payload["provenance"]["source"]["source_kind"],
                    "github_repository"
                );
                assert_eq!(
                    payload["provenance"]["source"]["source_id"],
                    "UbU-project/ubu-orchestrator"
                );
            }
            _ => {}
        }
    }

    assert_eq!(objective_count, 1);
    assert_eq!(preference_count, 3);
    assert_eq!(task_count, 1);
}

#[tokio::test]
async fn seed_rejects_second_run_without_duplicating_bootstrap_objects() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state.clone());

    app.clone()
        .oneshot(json_request(
            "/desktop/session/github-token",
            json!({
                "schema_version": DESKTOP_SESSION_SCHEMA_VERSION,
                "github_token": "desktop-session-test-value"
            }),
        ))
        .await
        .expect("token response");
    let first = app
        .clone()
        .oneshot(seed_request())
        .await
        .expect("first seed response");
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .clone()
        .oneshot(seed_request())
        .await
        .expect("second seed response");
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let body = json_body(second).await;
    assert_eq!(body["diagnostics"][0]["code"], "bootstrap_already_seeded");

    let row = sqlx::query(
        "SELECT COUNT(*) AS count FROM objects WHERE object_type IN ('Objective', 'Preference')",
    )
    .fetch_one(state.inner().store.pool())
    .await
    .expect("count query");
    let count: i64 = row.try_get("count").expect("count");
    assert_eq!(count, 4);
}

#[tokio::test]
async fn seed_requires_known_schema_version() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state);

    let response = app
        .oneshot(json_request(
            "/bootstrap/seed",
            json!({
                "selected_repo": {
                    "owner": "UbU-project",
                    "repo": "ubu-orchestrator"
                },
                "answers": {
                    "primary_objective": "Keep the orchestrator useful"
                }
            }),
        ))
        .await
        .expect("seed response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "missing_schema_version");
}

#[tokio::test]
async fn seed_without_token_does_not_admit_partial_bootstrap_state() {
    if std::env::var("GITHUB_TOKEN").is_ok() {
        return;
    }

    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .oneshot(seed_request())
        .await
        .expect("seed response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "missing_github_token");

    let row = sqlx::query("SELECT COUNT(*) AS count FROM objects")
        .fetch_one(state.inner().store.pool())
        .await
        .expect("count query");
    let count: i64 = row.try_get("count").expect("count");
    assert_eq!(count, 0);
}

fn seed_request() -> Request<Body> {
    json_request(
        "/bootstrap/seed",
        json!({
            "schema_version": BOOTSTRAP_SCHEMA_VERSION,
            "selected_repo": {
                "owner": "UbU-project",
                "repo": "ubu-orchestrator"
            },
            "answers": {
                "primary_objective": "Keep the orchestrator useful",
                "work_style": "focused",
                "planning_horizon_days": 7,
                "attention_preference": "deep_work"
            }
        }),
    )
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
    serde_json::from_slice(&bytes).expect("json body")
}
