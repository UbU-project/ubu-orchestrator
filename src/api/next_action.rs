use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::Result;
use crate::services::next_action_service;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextActionResponse {
    pub task_id: String,
    pub title: String,
    pub status: TaskLifecycleStatus,
    /// Computed response field; not a persisted task lifecycle status.
    pub readiness: bool,
    pub start: u64,
    pub end: u64,
}

#[utoipa::path(
    get,
    path = "/next-action",
    responses((status = 200, body = NextActionResponse))
)]
pub async fn next_action(State(state): State<AppState>) -> Result<Json<NextActionResponse>> {
    Ok(Json(next_action_service::get_next_action(state).await?))
}
