use axum::routing::{get, post};
use axum::Router;

use crate::api;
use crate::openapi;
use crate::state::AppState;

/*
Loopback risk: Phase 1 mutating HTTP endpoints are bound for local use, but
loopback-only binding is not a full security boundary. Local processes and
malicious local web content can still reach these endpoints while the temporary
HTTP bridge is present.

TODO(phase2-tauri-bridge): Per-run bearer-token and CSRF defenses are
intentionally deferred because this HTTP bridge is temporary pending the Tauri
command bridge.
*/
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(api::health::health))
        .route("/bootstrap/start", post(api::bootstrap::start))
        .route("/bootstrap/answer", post(api::bootstrap::answer))
        .route("/bootstrap/seed", post(api::bootstrap::seed))
        .route(
            "/desktop/session/github-token",
            post(api::desktop::github_token),
        )
        .route("/github/import/fixture", post(api::github::import_fixture))
        .route("/github/import/live", post(api::github::import_live))
        .route("/planning/generate", post(api::planning::generate))
        .route(
            "/planning/recalculate",
            post(api::recalculation::recalculate),
        )
        .route("/calendar/current", get(api::calendar::current))
        .route("/next-action", get(api::next_action::next_action))
        .route("/task/:task_id/start", post(api::user_action::start))
        .route(
            "/task/:task_id/action",
            post(api::user_action::record_action),
        )
        .route("/task/:task_id/done", post(api::user_action::done))
        .route("/task/:task_id/snooze", post(api::user_action::snooze))
        .route("/task/:task_id/reject", post(api::user_action::reject))
        .route(
            "/task/:task_id/decompose",
            post(api::user_action::decompose),
        )
        .route("/projection/preview", post(api::projection::preview))
        .route("/projection/approve", post(api::projection::approve))
        .route("/projection/reconcile", post(api::projection::reconcile))
        .route(
            "/projection/reconciliation/accept-external",
            post(api::projection::accept_external),
        )
        .route("/reports/risk", get(api::reports::risk))
        .route("/reports/human-complete", get(api::reports::human_complete))
        .route("/openapi.json", get(openapi::openapi_json))
        .with_state(state)
}
