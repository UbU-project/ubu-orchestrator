use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::test]
async fn health_returns_ok() {
    let app = ubu_orchestrator::build_router(AppState::new(ServerConfig::from_env()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
}
