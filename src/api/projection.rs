use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use ubu_core::AuthoritySource;
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::projection_service;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionPreviewRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionPreviewResponse {
    pub preview_id: String,
    pub operations: Vec<String>,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoritySourceBody {
    User,
    UserOverride,
    Delegated,
    AutomationWorker,
    Policy,
    System,
}

impl From<AuthoritySourceBody> for AuthoritySource {
    fn from(value: AuthoritySourceBody) -> Self {
        match value {
            AuthoritySourceBody::User => Self::User,
            AuthoritySourceBody::UserOverride => Self::UserOverride,
            AuthoritySourceBody::Delegated => Self::Delegated,
            AuthoritySourceBody::AutomationWorker => Self::AutomationWorker,
            AuthoritySourceBody::Policy => Self::Policy,
            AuthoritySourceBody::System => Self::System,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionApproveRequest {
    pub preview_id: String,
    #[serde(default = "default_authority_source")]
    pub authority_source: AuthoritySourceBody,
}

fn default_authority_source() -> AuthoritySourceBody {
    AuthoritySourceBody::User
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionResultResponse {
    pub preview_id: String,
    pub status: String,
    pub operation_results: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/projection/preview",
    request_body = ProjectionPreviewRequest,
    responses((status = 200, body = ProjectionPreviewResponse))
)]
pub async fn preview(
    State(state): State<AppState>,
    Json(request): Json<ProjectionPreviewRequest>,
) -> Result<Json<ProjectionPreviewResponse>> {
    Ok(Json(projection_service::preview(state, request).await?))
}

#[utoipa::path(
    post,
    path = "/projection/approve",
    request_body = ProjectionApproveRequest,
    responses((status = 200, body = ProjectionResultResponse))
)]
// TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
// left without per-run bearer-token or CSRF defenses while the temporary HTTP
// bridge remains in Phase 1.
pub async fn approve(
    State(state): State<AppState>,
    Json(request): Json<ProjectionApproveRequest>,
) -> Result<Json<ProjectionResultResponse>> {
    Ok(Json(projection_service::approve(state, request).await?))
}
