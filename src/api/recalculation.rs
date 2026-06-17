use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::planning::{DiagnosticBody, PlanBody, RepairScopeBody};
use crate::errors::Result;
use crate::services::recalculation_service;
use crate::state::AppState;

pub const RECALCULATION_SCHEMA_VERSION: &str = "ubu.orchestrator.recalculation.v1";

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RecalculationRequest {
    #[serde(default)]
    pub schema_version: Option<String>,
    pub triggered_at: String,
    pub trigger_type: RecalculationTriggerTypeBody,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub objects: Vec<ObjectRefBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ObjectRefBody {
    pub id: String,
    pub object_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecalculationTriggerTypeBody {
    TaskCompleted,
    TaskFailed,
    TaskMoot,
    UserOverride,
    ObservedSnapshot,
    ExternalEvent,
    GithubUpdate,
    LowCompactCalendarCoverage,
    WorkerRequest,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RecalculationResponse {
    pub schema_version: String,
    pub trigger_type: RecalculationTriggerTypeBody,
    pub repair_scope: RepairScopeBody,
    pub prior_plan_id: String,
    pub plan: Option<PlanBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[utoipa::path(
    post,
    path = "/planning/recalculate",
    request_body = RecalculationRequest,
    responses((status = 200, body = RecalculationResponse))
)]
pub async fn recalculate(
    State(state): State<AppState>,
    Json(request): Json<RecalculationRequest>,
) -> Result<Json<RecalculationResponse>> {
    Ok(Json(
        recalculation_service::recalculate_from_request(state, request).await?,
    ))
}
