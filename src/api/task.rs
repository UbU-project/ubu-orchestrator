use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::{
    errors::{AppError, Result},
    services::{task_capture, task_read},
    state::AppState,
};

pub const TASK_CAPTURE_SCHEMA_VERSION: &str = "ubu.orchestrator.task_capture.v1";
pub const TASK_READ_SCHEMA_VERSION: &str = "ubu.orchestrator.task_read.v1";

#[derive(Debug, Deserialize, ToSchema)]
pub struct CaptureRequest {
    pub schema_version: Option<String>,
    #[serde(flatten)]
    pub fields: Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditRequest {
    pub schema_version: Option<String>,
    pub expected_version: i64,
    #[serde(flatten)]
    pub fields: Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskWriteResponse {
    pub schema_version: String,
    pub task_id: String,
    pub version: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct TaskReadQuery {
    /// A read has no body; when supplied, it must name the read contract.
    pub schema_version: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct TaskListQuery {
    pub schema_version: Option<String>,
    /// One of active, completed, failed, moot. Defaults to active.
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPlacement {
    Static,
    Planned,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskSummary {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub version: i64,
    pub placement: TaskPlacement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_estimate: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_tag: Option<String>,
    /// Occurrences are listed but rejected by `PATCH /task/{task_id}`.
    pub is_routine_occurrence: bool,
    /// Present only for a child of an active Container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskListResponse {
    pub schema_version: String,
    pub status: String,
    pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskReadResponse {
    pub schema_version: String,
    pub task_id: String,
    pub version: i64,
    pub status: String,
    pub is_routine_occurrence: bool,
    pub payload: Value,
}

fn validate_read_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(TASK_READ_SCHEMA_VERSION) | None => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
    }
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(TASK_CAPTURE_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

#[utoipa::path(
    post,
    path = "/task",
    request_body = CaptureRequest,
    responses((status = 201, body = TaskWriteResponse))
)]
pub async fn capture(
    State(state): State<AppState>,
    Json(request): Json<CaptureRequest>,
) -> Result<(StatusCode, Json<TaskWriteResponse>)> {
    validate_schema_version(request.schema_version.as_deref())?;
    let (task_id, version) = task_capture::capture(&state, request.fields).await?;
    Ok((
        StatusCode::CREATED,
        Json(TaskWriteResponse {
            schema_version: TASK_CAPTURE_SCHEMA_VERSION.into(),
            task_id,
            version,
        }),
    ))
}

#[utoipa::path(
    patch,
    path = "/task/{task_id}",
    params(("task_id" = String, Path)),
    request_body = EditRequest,
    responses((status = 200, body = TaskWriteResponse))
)]
pub async fn edit(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<EditRequest>,
) -> Result<Json<TaskWriteResponse>> {
    validate_schema_version(request.schema_version.as_deref())?;
    let (task_id, version) =
        task_capture::edit(&state, &task_id, request.expected_version, request.fields).await?;
    Ok(Json(TaskWriteResponse {
        schema_version: TASK_CAPTURE_SCHEMA_VERSION.into(),
        task_id,
        version,
    }))
}

#[utoipa::path(
    get,
    path = "/task/{task_id}",
    params(("task_id" = String, Path), TaskReadQuery),
    responses((status = 200, body = TaskReadResponse), (status = 404, description = "Unknown Task"))
)]
pub async fn read(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Query(query): Query<TaskReadQuery>,
) -> Result<Json<TaskReadResponse>> {
    validate_read_schema_version(query.schema_version.as_deref())?;
    Ok(Json(task_read::get(&state, &task_id).await?))
}

#[utoipa::path(
    get,
    path = "/tasks",
    params(TaskListQuery),
    responses((status = 200, body = TaskListResponse), (status = 400, description = "Unknown status"))
)]
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<TaskListResponse>> {
    validate_read_schema_version(query.schema_version.as_deref())?;
    Ok(Json(
        task_read::list(&state, query.status.as_deref()).await?,
    ))
}
