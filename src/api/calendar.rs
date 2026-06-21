use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::planning::{
    LegitimizationReportBody, PlanCandidateBody, ProbabilityQualityBody, ScheduledTaskBody,
};
use crate::errors::Result;
use crate::services::planning_service;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct CalendarResponse {
    pub plan_id: Option<String>,
    pub steps: Vec<ScheduledTaskBody>,
    pub display_probability: Option<f64>,
    pub probability_interval_low: Option<f64>,
    pub probability_interval_high: Option<f64>,
    pub robustness_score: Option<f64>,
    pub probability_quality: ProbabilityQualityBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legitimization: Option<LegitimizationReportBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_candidate: Option<PlanCandidateBody>,
    pub alternatives: Vec<PlanCandidateBody>,
}

#[utoipa::path(
    get,
    path = "/calendar/current",
    responses((status = 200, body = CalendarResponse))
)]
pub async fn current(State(state): State<AppState>) -> Result<Json<CalendarResponse>> {
    Ok(Json(planning_service::current_calendar(state).await?))
}
