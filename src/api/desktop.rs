use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::desktop_session_service;
use crate::state::AppState;

pub const DESKTOP_SESSION_SCHEMA_VERSION: &str = "ubu.orchestrator.desktop_session.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct GithubTokenIntakeRequest {
    #[serde(default)]
    pub schema_version: Option<String>,
    pub github_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct GithubTokenIntakeResponse {
    pub schema_version: String,
    pub accepted: bool,
    pub token_available: bool,
}

#[utoipa::path(
    post,
    path = "/desktop/session/github-token",
    request_body = GithubTokenIntakeRequest,
    responses((status = 200, body = GithubTokenIntakeResponse))
)]
// TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
// left without per-run bearer-token or CSRF defenses while the temporary HTTP
// bridge remains in Phase 1.
pub async fn github_token(
    State(state): State<AppState>,
    Json(request): Json<GithubTokenIntakeRequest>,
) -> Result<Json<GithubTokenIntakeResponse>> {
    let response = desktop_session_service::github_token(state, request).await?;
    Ok(Json(response))
}
