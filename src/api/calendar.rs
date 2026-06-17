use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::planning::ScheduledTaskBody;
use crate::errors::Result;
use crate::services::planning_service;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct CalendarResponse {
    pub plan_id: Option<String>,
    pub steps: Vec<ScheduledTaskBody>,
}

#[utoipa::path(
    get,
    path = "/calendar/current",
    responses((status = 200, body = CalendarResponse))
)]
pub async fn current(State(state): State<AppState>) -> Result<Json<CalendarResponse>> {
    Ok(Json(planning_service::current_calendar(state).await?))
}
