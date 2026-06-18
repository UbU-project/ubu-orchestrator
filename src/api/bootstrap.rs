use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::bootstrap_service;
use crate::state::AppState;

pub const BOOTSTRAP_SCHEMA_VERSION: &str = "ubu.orchestrator.bootstrap.v1";

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapStartResponse {
    pub started: bool,
    pub next_prompt: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapAnswerRequest {
    pub answer: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapAnswerResponse {
    pub accepted: bool,
    pub answer_count: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapSeedRequest {
    #[serde(default)]
    pub schema_version: Option<String>,
    pub selected_repo: BootstrapSelectedRepo,
    pub answers: BootstrapAnswers,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapSelectedRepo {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapAnswers {
    pub primary_objective: String,
    #[serde(default = "default_work_style")]
    pub work_style: WorkStyle,
    #[serde(default = "default_planning_horizon_days")]
    pub planning_horizon_days: u8,
    #[serde(default = "default_attention_preference")]
    pub attention_preference: AttentionPreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptable_energy_floor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerable_stress_ceiling: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerable_intensity_ceiling: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkStyle {
    Focused,
    Balanced,
    Responsive,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttentionPreference {
    DeepWork,
    Mixed,
    QuickTurnaround,
}

fn default_work_style() -> WorkStyle {
    WorkStyle::Balanced
}

fn default_planning_horizon_days() -> u8 {
    7
}

fn default_attention_preference() -> AttentionPreference {
    AttentionPreference::Mixed
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapSeedResponse {
    pub schema_version: String,
    pub objective_ids: Vec<String>,
    pub preference_ids: Vec<String>,
    pub imported_tasks: crate::api::github::ImportResponse,
    pub diagnostics: Vec<BootstrapDiagnostic>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapDiagnostic {
    pub code: String,
    pub message: String,
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

#[utoipa::path(
    post,
    path = "/bootstrap/seed",
    request_body = BootstrapSeedRequest,
    responses((status = 200, body = BootstrapSeedResponse))
)]
// TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
// left without per-run bearer-token or CSRF defenses while the temporary HTTP
// bridge remains in Phase 1.
pub async fn seed(
    State(state): State<AppState>,
    Json(request): Json<BootstrapSeedRequest>,
) -> Result<Json<BootstrapSeedResponse>> {
    let response = bootstrap_service::seed(state, request).await?;
    Ok(Json(response))
}
