use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::reports::{HumanCompletePlanQualityResponse, RiskReportResponse};
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::Result;
use crate::services::next_action_service;
use crate::state::AppState;

pub const NEXT_ACTION_SCHEMA_VERSION: &str = "ubu.orchestrator.next_action.v1";

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionRequest {
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionResponse {
    pub schema_version: String,
    pub recommendation: Option<NextActionRecommendation>,
    pub diagnostics: Vec<NextActionDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_report: Option<RiskReportResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_complete_plan_quality: Option<HumanCompletePlanQualityResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionRecommendation {
    pub task_id: String,
    pub title: String,
    pub status: TaskLifecycleStatus,
    /// Computed response field; not a persisted task lifecycle status.
    pub readiness: ReadinessState,
    pub parent_objective: Option<NextActionObjectiveRef>,
    pub source_refs: Vec<NextActionSourceRef>,
    pub selection: NextActionSelection,
    pub explanation: NextActionExplanation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionObjectiveRef {
    pub objective_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionSourceRef {
    pub source_kind: String,
    pub source_id: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionSelection {
    pub rule: String,
    pub priority: Option<i64>,
    pub tiebreak: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionExplanation {
    pub template_id: String,
    pub label: String,
    pub message: String,
    pub readiness_state: ReadinessState,
    pub parent_objective: Option<NextActionObjectiveRef>,
    pub source_refs: Vec<NextActionSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionDiagnostic {
    pub code: NextActionDiagnosticCode,
    pub message: String,
    pub blocked_task_count: usize,
    pub sampled_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NextActionDiagnosticCode {
    NoAdmittedTasks,
    NoActiveTasks,
    AllCandidatesBlockedOnUnmetDependencies,
    AllCandidatesBlockedOnPreconditions,
    NoReadyTask,
}

#[utoipa::path(
    get,
    path = "/next-action",
    params(("schema_version" = Option<String>, Query)),
    responses((status = 200, body = NextActionResponse))
)]
pub async fn next_action(
    State(state): State<AppState>,
    Query(request): Query<NextActionRequest>,
) -> Result<Json<NextActionResponse>> {
    Ok(Json(
        next_action_service::get_next_action(state, request).await?,
    ))
}
