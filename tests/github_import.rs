use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use ubu_orchestrator::api::desktop::DESKTOP_SESSION_SCHEMA_VERSION;
use ubu_orchestrator::config::{GithubIngestMode, ServerConfig};
use ubu_orchestrator::state::AppState;

#[tokio::test]
async fn default_mock_import_uses_recording_api_without_token() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .oneshot(import_request())
        .await
        .expect("import response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["imported"], 1);
    assert_eq!(body["admitted_to_store"], 2);
    assert_eq!(body["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(body["candidates"][0]["source"], "github_issue");

    let external_reference_count: i64 =
        sqlx::query("SELECT COUNT(*) AS count FROM external_references")
            .fetch_one(state.inner().store.pool())
            .await
            .expect("external reference count")
            .try_get("count")
            .expect("count");
    assert_eq!(external_reference_count, 1);
}

#[tokio::test]
async fn token_presence_alone_does_not_enable_live_ingest() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let app = ubu_orchestrator::build_router(state);

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
        .oneshot(import_request())
        .await
        .expect("import response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["candidates"][0]["source"], "github_issue");
}

#[tokio::test]
async fn live_ingest_without_token_fails_without_writes() {
    if std::env::var("GITHUB_TOKEN").is_ok() {
        return;
    }

    let state = AppState::in_memory(
        ServerConfig::from_env().with_github_ingest_mode(GithubIngestMode::Live),
    )
    .await
    .expect("state");
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .oneshot(import_request())
        .await
        .expect("import response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(
        body["diagnostics"][0]["code"],
        "missing_github_ingest_token"
    );

    let object_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM objects")
        .fetch_one(state.inner().store.pool())
        .await
        .expect("object count")
        .try_get("count")
        .expect("count");
    let external_reference_count: i64 =
        sqlx::query("SELECT COUNT(*) AS count FROM external_references")
            .fetch_one(state.inner().store.pool())
            .await
            .expect("external reference count")
            .try_get("count")
            .expect("count");
    assert_eq!(object_count, 0);
    assert_eq!(external_reference_count, 0);
}

fn import_request() -> Request<Body> {
    json_request(
        "/github/import/live",
        json!({
            "owner": "UbU-project",
            "repo": "ubu-orchestrator"
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
