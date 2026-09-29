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
    /// Record that work began without changing lifecycle state or applying effects.
    Start,
    Complete,
    Skip,
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

/// Undo of a completion, naming the completion it undoes.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReopenRequest {
    pub schema_version: Option<String>,
    /// The `log_id` the completion returned. It must be the Task's latest completion.
    pub completion_log_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReopenResponse {
    pub schema_version: String,
    /// The reopen decision that was recorded.
    pub log_id: String,
    pub task_id: String,
    pub completion_log_id: String,
    pub task_status: TaskLifecycleStatus,
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
    path = "/task/{task_id}/reopen",
    params(("task_id" = String, Path)),
    request_body = ReopenRequest,
    responses((status = 200, body = ReopenResponse), (status = 400, description = "Unknown or missing schema version"), (status = 409, description = "Not completed, no completion recorded, or not the latest completion"))
)]
pub async fn reopen(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ReopenRequest>,
) -> Result<Json<ReopenResponse>> {
    log_service::validate_schema_version(request.schema_version.as_deref())?;
    Ok(Json(
        log_service::reopen_completion(&state, &task_id, &request.completion_log_id).await?,
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
    request_body = crate::api::container::DecomposeRequest,
    responses((status = 200, body = crate::api::container::DecomposeResponse))
)]
pub async fn decompose(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<crate::api::container::DecomposeRequest>,
) -> Result<Json<crate::api::container::DecomposeResponse>> {
    Ok(Json(
        crate::services::decomposition::decompose(&state, &task_id, request).await?,
    ))
}
