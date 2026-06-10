use axum::routing::{get, post};
use axum::Router;

use crate::api;
use crate::openapi;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(api::health::health))
        .route("/bootstrap/start", post(api::bootstrap::start))
        .route("/bootstrap/answer", post(api::bootstrap::answer))
        .route("/github/import/fixture", post(api::github::import_fixture))
        .route("/github/import/live", post(api::github::import_live))
        .route("/planning/generate", post(api::planning::generate))
        .route("/calendar/current", get(api::calendar::current))
        .route("/next-action", get(api::next_action::next_action))
        .route("/task/:task_id/start", post(api::user_action::start))
        .route("/task/:task_id/done", post(api::user_action::done))
        .route("/task/:task_id/snooze", post(api::user_action::snooze))
        .route("/task/:task_id/reject", post(api::user_action::reject))
        .route(
            "/task/:task_id/decompose",
            post(api::user_action::decompose),
        )
        .route("/projection/preview", post(api::projection::preview))
        .route("/projection/approve", post(api::projection::approve))
        .route("/reports/risk", get(api::reports::risk))
        .route("/reports/human-complete", get(api::reports::human_complete))
        .route("/openapi.json", get(openapi::openapi_json))
        .with_state(state)
}
