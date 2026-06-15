use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::import_service;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportFixtureRequest {
    #[serde(default = "default_fixture_path")]
    pub fixture_path: String,
}

fn default_fixture_path() -> String {
    "fixtures/fixture-loop/github-small.json".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportLiveRequest {
    pub owner: String,
    pub repo: String,
    #[serde(default)]
    pub session_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportedCandidate {
    pub task_id: String,
    pub title: String,
    pub source: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub imported: usize,
    pub admitted_to_store: usize,
    pub candidates: Vec<ImportedCandidate>,
}

#[utoipa::path(
    post,
    path = "/github/import/fixture",
    request_body = ImportFixtureRequest,
    responses((status = 200, body = ImportResponse))
)]
pub async fn import_fixture(
    State(state): State<AppState>,
    Json(request): Json<ImportFixtureRequest>,
) -> Result<Json<ImportResponse>> {
    let response = import_service::import_fixture(state, request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/github/import/live",
    request_body = ImportLiveRequest,
    responses((status = 200, body = ImportResponse))
)]
// TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
// left without per-run bearer-token or CSRF defenses while the temporary HTTP
// bridge remains in Phase 1.
pub async fn import_live(
    State(state): State<AppState>,
    Json(request): Json<ImportLiveRequest>,
) -> Result<Json<ImportResponse>> {
    let response = import_service::import_live(state, request).await?;
    Ok(Json(response))
}
