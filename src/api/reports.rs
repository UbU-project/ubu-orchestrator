use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
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
    RoutineTriage,
    UnplacedWork,
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

pub const TIME_BY_CATEGORY_SCHEMA_VERSION: &str = "ubu.orchestrator.time_by_category.v1";

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct TimeByCategoryQuery {
    pub schema_version: Option<String>,
    /// Inclusive RFC 3339 start. Absent: seven days before `to`.
    pub from: Option<String>,
    /// Inclusive RFC 3339 end. Absent: now.
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct CategoryTime {
    pub category: String,
    /// `static_seconds + completed_seconds`.
    pub seconds: u64,
    /// Overlap of Static windows with the range, whether or not the Task was completed.
    pub static_seconds: u64,
    /// Dynamic Tasks completed inside the range, by observed window or estimate.
    pub completed_seconds: u64,
    /// Tasks that contributed to this row.
    pub task_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct UnmeasuredTask {
    pub task_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TimeByCategoryResponse {
    pub schema_version: String,
    pub generated_at: String,
    pub from: String,
    pub to: String,
    /// Sorted by `seconds` descending, then `category` ascending.
    pub categories: Vec<CategoryTime>,
    /// Work that happened in the range and could not be measured.
    pub unmeasured: Vec<UnmeasuredTask>,
    pub total_seconds: u64,
}

#[utoipa::path(
    get,
    path = "/reports/time-by-category",
    params(TimeByCategoryQuery),
    responses((status = 200, body = TimeByCategoryResponse), (status = 400, description = "Missing or unknown schema version, an unreadable bound, or from after to"))
)]
pub async fn time_by_category(
    State(state): State<AppState>,
    Query(query): Query<TimeByCategoryQuery>,
) -> Result<Json<TimeByCategoryResponse>> {
    match query.schema_version.as_deref() {
        Some(TIME_BY_CATEGORY_SCHEMA_VERSION) => {}
        Some(other) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                format!("unsupported schema_version `{other}`"),
            ))
        }
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
    }
    Ok(Json(report_service::time_by_category(&state, query.from.as_deref(), query.to.as_deref()).await?))
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
