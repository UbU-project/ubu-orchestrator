//! Explicit, on-demand Calendar capture.
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    services::{calendar_capture, calendar_client::CalendarExportMode},
    state::AppState,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const CALENDAR_CAPTURE_SCHEMA_VERSION: &str = "ubu.orchestrator.calendar_capture.v1";
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarCaptureRequest {
    pub schema_version: Option<String>,
    pub export_mode: CalendarExportMode,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarCaptureResponse {
    pub schema_version: String,
    pub captured: usize,
    /// Tasks whose canonical state actually changed.
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub diagnostics: Vec<DiagnosticBody>,
}
#[utoipa::path(post, path = "/projection/calendar/capture", request_body = CalendarCaptureRequest,
    responses((status = 200, body = CalendarCaptureResponse),
        (status = 400, description = "Missing or unsupported schema"),
        (status = 403, description = "Live Calendar is not enabled"),
        (status = 503, description = "Google credential paths are not configured"),
        (status = 502, description = "Calendar observation failed")))]
pub async fn capture(
    State(state): State<AppState>,
    Json(request): Json<CalendarCaptureRequest>,
) -> Result<Json<CalendarCaptureResponse>> {
    match request.schema_version.as_deref() {
        Some(CALENDAR_CAPTURE_SCHEMA_VERSION) => {}
        Some(_) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                "unsupported Calendar capture schema_version",
            ))
        }
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
    }
    Ok(Json(
        calendar_capture::capture(&state, request.export_mode).await?,
    ))
}
