use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::bootstrap_service;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStartResponse {
    pub started: bool,
    pub next_prompt: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapAnswerRequest {
    pub answer: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapAnswerResponse {
    pub accepted: bool,
    pub answer_count: usize,
}

#[utoipa::path(
    post,
    path = "/bootstrap/start",
    responses((status = 200, body = BootstrapStartResponse))
)]
pub async fn start(State(state): State<AppState>) -> Result<Json<BootstrapStartResponse>> {
    let response = bootstrap_service::start(state).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/bootstrap/answer",
    request_body = BootstrapAnswerRequest,
    responses((status = 200, body = BootstrapAnswerResponse))
)]
pub async fn answer(
    State(state): State<AppState>,
    Json(request): Json<BootstrapAnswerRequest>,
) -> Result<Json<BootstrapAnswerResponse>> {
    let response = bootstrap_service::answer(state, request).await?;
    Ok(Json(response))
}
