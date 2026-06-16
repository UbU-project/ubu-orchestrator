use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use ubu_core::AuthoritySource;
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::projection_service;
use crate::state::AppState;

pub const PROJECTION_PREVIEW_SCHEMA_VERSION: &str = "ubu.orchestrator.projection_preview.v1";
pub const PROJECTION_APPROVAL_SCHEMA_VERSION: &str = "ubu.orchestrator.projection_approval.v1";
pub const PROJECTION_RESULT_SCHEMA_VERSION: &str = "ubu.orchestrator.projection_result.v1";
pub const PROJECTION_RECONCILIATION_SCHEMA_VERSION: &str =
    "ubu.orchestrator.projection_reconciliation.v1";
pub const PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION: &str =
    "ubu.orchestrator.projection_external_accept.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionPreviewRequest {
    pub schema_version: Option<String>,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default = "default_repo")]
    pub repo: String,
    #[serde(default)]
    pub issue_number: Option<u64>,
    #[serde(default)]
    pub observed_labels: Vec<String>,
    #[serde(default = "default_desired_labels")]
    pub desired_labels: Vec<String>,
    #[serde(default)]
    pub existing_repository_labels: Vec<String>,
    #[serde(default)]
    pub no_external_export: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

fn default_owner() -> String {
    "UbU-project".to_owned()
}

fn default_repo() -> String {
    "ubu-orchestrator".to_owned()
}

fn default_desired_labels() -> Vec<String> {
    vec!["ubu-managed".to_owned()]
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionPreviewResponse {
    pub schema_version: String,
    pub preview_id: String,
    pub operations: Vec<ProjectionOperationBody>,
    pub policy_summary: PolicySummaryBody,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionOperationBody {
    pub operation_id: String,
    pub kind: String,
    pub target: ProjectionTargetBody,
    pub summary: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionTargetBody {
    pub owner: String,
    pub repo: String,
    pub issue_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PolicySummaryBody {
    pub legitimization: String,
    pub adjudication_reasons: Vec<String>,
    pub local_only: Option<bool>,
    pub no_cloud_llm: Option<bool>,
    pub no_external_export: Option<bool>,
    pub checked_at: String,
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
    pub schema_version: Option<String>,
    pub preview_id: String,
    pub approved: bool,
    #[serde(default = "default_authority_source")]
    pub authority_source: AuthoritySourceBody,
    pub approved_at: Option<String>,
}

fn default_authority_source() -> AuthoritySourceBody {
    AuthoritySourceBody::User
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionResultResponse {
    pub schema_version: String,
    pub preview_id: String,
    pub status: String,
    pub operation_results: Vec<ProjectionOperationResultBody>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionOperationResultBody {
    pub operation_id: String,
    pub status: String,
    pub message: Option<String>,
    pub authority_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionDiagnostic {
    pub code: String,
    pub message: String,
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionReconcileRequest {
    pub schema_version: Option<String>,
    #[serde(default)]
    pub observed_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionReconcileResponse {
    pub schema_version: String,
    pub reconciliation_id: String,
    pub preview_id: String,
    pub status: String,
    pub conflicts: Vec<ProjectionConflictBody>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionConflictBody {
    pub operation_id: String,
    pub conflict_type: String,
    pub expected_label: String,
    pub observed_labels: Vec<String>,
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionAcceptExternalRequest {
    pub schema_version: Option<String>,
    pub reconciliation_id: String,
    pub conflict_operation_id: String,
    #[serde(default = "default_authority_source")]
    pub authority_source: AuthoritySourceBody,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProjectionAcceptExternalResponse {
    pub schema_version: String,
    pub admitted_object_id: String,
    pub reconciliation_id: String,
    pub conflict_operation_id: String,
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

#[utoipa::path(
    post,
    path = "/projection/reconcile",
    request_body = ProjectionReconcileRequest,
    responses((status = 200, body = ProjectionReconcileResponse))
)]
pub async fn reconcile(
    State(state): State<AppState>,
    Json(request): Json<ProjectionReconcileRequest>,
) -> Result<Json<ProjectionReconcileResponse>> {
    Ok(Json(projection_service::reconcile(state, request).await?))
}

#[utoipa::path(
    post,
    path = "/projection/reconciliation/accept-external",
    request_body = ProjectionAcceptExternalRequest,
    responses((status = 200, body = ProjectionAcceptExternalResponse))
)]
// TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
// left without per-run bearer-token or CSRF defenses while the temporary HTTP
// bridge remains in Phase 1.
pub async fn accept_external(
    State(state): State<AppState>,
    Json(request): Json<ProjectionAcceptExternalRequest>,
) -> Result<Json<ProjectionAcceptExternalResponse>> {
    Ok(Json(
        projection_service::accept_external(state, request).await?,
    ))
}
