use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::test]
async fn fixture_loop_reaches_projection_result() {
    let app = ubu_orchestrator::build_router(AppState::new(ServerConfig::from_env()));

    let response = app
        .clone()
        .oneshot(json_request("/bootstrap/start", "{}"))
        .await
        .expect("bootstrap response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "/github/import/fixture",
            r#"{"fixture_path":"fixtures/fixture-loop/github-small.json"}"#,
        ))
        .await
        .expect("fixture import response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", "{}"))
        .await
        .expect("planning response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request("/task/issue-1/done", r#"{"note":"done"}"#))
        .await
        .expect("done response");
    assert_eq!(response.status(), StatusCode::OK);

    let preview_response = app
        .clone()
        .oneshot(json_request("/projection/preview", "{}"))
        .await
        .expect("preview response");
    assert_eq!(preview_response.status(), StatusCode::OK);

    let preview_body = preview_response
        .into_body()
        .collect()
        .await
        .expect("preview body")
        .to_bytes();
    let preview: Value = serde_json::from_slice(&preview_body).expect("preview json");
    let preview_id = preview
        .get("preview_id")
        .and_then(Value::as_str)
        .expect("preview id");

    let approve_body = format!(r#"{{"preview_id":"{preview_id}","authority_source":"user"}}"#);
    let approve_response = app
        .oneshot(json_request("/projection/approve", &approve_body))
        .await
        .expect("approve response");
    assert_eq!(approve_response.status(), StatusCode::OK);
}

fn json_request(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request")
}
