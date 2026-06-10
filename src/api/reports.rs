use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::report_service;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RiskReportResponse {
    pub risks: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HumanCompleteReportResponse {
    pub completed_tasks: usize,
    pub notes: Vec<String>,
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
