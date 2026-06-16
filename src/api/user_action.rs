use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::log_service;
use crate::state::AppState;

pub const TASK_ACTION_SCHEMA_VERSION: &str = "ubu.orchestrator.task_action.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskActionKind {
    Start,
    Done,
    Snooze,
    Reject,
    Decompose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycleStatus {
    Active,
    Completed,
    Failed,
    Moot,
}

impl TaskLifecycleStatus {
    pub fn for_action(action: TaskActionKind) -> Self {
        match action {
            TaskActionKind::Start | TaskActionKind::Snooze | TaskActionKind::Decompose => {
                Self::Active
            }
            TaskActionKind::Done => Self::Completed,
            TaskActionKind::Reject => Self::Moot,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct UserActionRequest {
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordedTaskActionKind {
    Complete,
    Override,
    Snooze,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RecordedTaskActionRequest {
    pub schema_version: Option<String>,
    pub action: RecordedTaskActionKind,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RecordedTaskActionResponse {
    pub schema_version: String,
    pub log_id: String,
    pub task_id: String,
    pub action: RecordedTaskActionKind,
    pub task_status: TaskLifecycleStatus,
    pub authority_source: String,
    pub transition_applied: bool,
    pub note: Option<String>,
    pub diagnostics: Vec<ActionDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ActionDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct LogEntryResponse {
    pub log_id: String,
    pub task_id: String,
    pub action: TaskActionKind,
    pub status: TaskLifecycleStatus,
    pub authority_source: String,
    pub note: Option<String>,
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/start",
    params(("task_id" = String, Path)),
    request_body = UserActionRequest,
    responses((status = 200, body = LogEntryResponse))
)]
pub async fn start(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<UserActionRequest>,
) -> Result<Json<LogEntryResponse>> {
    Ok(Json(
        log_service::append_action(state, task_id, TaskActionKind::Start, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/action",
    params(("task_id" = String, Path)),
    request_body = RecordedTaskActionRequest,
    responses((status = 200, body = RecordedTaskActionResponse))
)]
pub async fn record_action(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<RecordedTaskActionRequest>,
) -> Result<Json<RecordedTaskActionResponse>> {
    Ok(Json(
        log_service::record_task_action(state, task_id, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/done",
    params(("task_id" = String, Path)),
    request_body = UserActionRequest,
    responses((status = 200, body = LogEntryResponse))
)]
pub async fn done(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<UserActionRequest>,
) -> Result<Json<LogEntryResponse>> {
    Ok(Json(
        log_service::append_action(state, task_id, TaskActionKind::Done, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/snooze",
    params(("task_id" = String, Path)),
    request_body = UserActionRequest,
    responses((status = 200, body = LogEntryResponse))
)]
pub async fn snooze(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<UserActionRequest>,
) -> Result<Json<LogEntryResponse>> {
    Ok(Json(
        log_service::append_action(state, task_id, TaskActionKind::Snooze, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/reject",
    params(("task_id" = String, Path)),
    request_body = UserActionRequest,
    responses((status = 200, body = LogEntryResponse))
)]
pub async fn reject(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<UserActionRequest>,
) -> Result<Json<LogEntryResponse>> {
    Ok(Json(
        log_service::append_action(state, task_id, TaskActionKind::Reject, request).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/task/{task_id}/decompose",
    params(("task_id" = String, Path)),
    request_body = UserActionRequest,
    responses((status = 200, body = LogEntryResponse))
)]
pub async fn decompose(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<UserActionRequest>,
) -> Result<Json<LogEntryResponse>> {
    Ok(Json(
        log_service::append_action(state, task_id, TaskActionKind::Decompose, request).await?,
    ))
}
