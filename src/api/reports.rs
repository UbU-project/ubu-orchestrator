use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::Result;
use crate::services::report_service;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RiskReportResponse {
    pub risks: Vec<String>,
    pub task_statuses: Vec<TaskStatusCount>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct HumanCompleteReportResponse {
    pub completed_tasks: usize,
    pub task_statuses: Vec<TaskStatusCount>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TaskStatusCount {
    pub status: TaskLifecycleStatus,
    pub count: usize,
}

#[utoipa::path(
    get,
    path = "/reports/risk",
    responses((status = 200, body = RiskReportResponse))
)]
pub async fn risk(State(state): State<AppState>) -> Result<Json<RiskReportResponse>> {
    Ok(Json(report_service::risk(state).await?))
}

#[utoipa::path(
    get,
    path = "/reports/human-complete",
    responses((status = 200, body = HumanCompleteReportResponse))
)]
pub async fn human_complete(
    State(state): State<AppState>,
) -> Result<Json<HumanCompleteReportResponse>> {
    Ok(Json(report_service::human_complete(state).await?))
}
