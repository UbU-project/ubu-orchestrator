use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::{
    errors::{AppError, Result},
    services::task_capture,
    state::AppState,
};

pub const TASK_CAPTURE_SCHEMA_VERSION: &str = "ubu.orchestrator.task_capture.v1";

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
