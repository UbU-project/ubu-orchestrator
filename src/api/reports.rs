use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::Result;
use crate::services::report_service;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RiskReportResponse {
    pub generated_at: String,
    pub level: RiskLevel,
    pub findings: Vec<RiskFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RiskFinding {
    pub category: RiskCategory,
    pub severity: RiskLevel,
    pub blocking: bool,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskCategory {
    DeadlineRisk,
    DependencyFragility,
    WorkerBottleneck,
    StaleAffect,
    AffectMargin,
    DestructivePressure,
    PostPlanDepletion,
    LowCoverage,
    SkeletonFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct HumanCompletePlanQualityResponse {
    pub generated_at: String,
    pub plan_ref: String,
    pub feedback_latency: u64,
    pub checkpoint_coverage: CheckpointCoverage,
    pub affect_margin: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violated_dimensions: Vec<String>,
    pub failure_pattern: FailurePattern,
    pub stretch_pressure: StretchPressure,
    pub post_plan_state_delta: PostPlanStateDelta,
    pub revision_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointCoverage {
    Adequate,
    Sparse,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailurePattern {
    None,
    WrongEstimates,
    MissingDependencies,
    StaleAffect,
    Interruption,
    Overload,
    ChangedObjective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StretchPressure {
    Comfort,
    SustainableStretch,
    DestructivePressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostPlanStateDelta {
    Better,
    Neutral,
    Depleted,
    AtRisk,
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
