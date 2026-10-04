use axum::{extract::{Query, State}, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    api::planning::DiagnosticBody,
    errors::Result,
    services::{
        calendar_projection::{CalendarOperation, DesiredEvent},
        calendar_apply,
    },
    state::AppState,
};

pub const CALENDAR_PROJECTION_PREVIEW_SCHEMA_VERSION: &str =
    "ubu.orchestrator.calendar_projection_preview.v1";

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarEventBody {
    pub external_id: String,
    pub task_id: String,
    pub summary: String,
    pub start_at: String,
    pub end_at: String,
    pub color_id: Option<String>,
    pub transparent: bool,
    pub reminders_minutes: Vec<i64>,
}

impl From<DesiredEvent> for CalendarEventBody {
    fn from(event: DesiredEvent) -> Self {
        Self {
            external_id: event.external_id,
            task_id: event.task_id,
            summary: event.summary,
            start_at: event.start_at,
            end_at: event.end_at,
            color_id: event.color_id,
            transparent: event.transparent,
            reminders_minutes: event.reminders_minutes,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalendarOperationBody {
    Create {
        event: CalendarEventBody,
        /// Whether the event's Task is Static: placed at a window of its own, and
        /// not packed by the planner. Read from the Plan step, or from the Task
        /// record when the Plan no longer holds it. **Never inferred from the
        /// colour**: a Static Task with no category has no colour, and a colour
        /// on a Dynamic event means done.
        static_anchor: bool,
    },
    Update {
        event: CalendarEventBody,
        /// As on `create`.
        static_anchor: bool,
    },
    Delete {
        external_id: String,
        summary: String,
    },
}

impl CalendarOperationBody {
    /// `is_static` answers for a Task id. It is the caller's knowledge of the
    /// Plan and the store; an operation alone does not carry its placement.
    pub fn from_operation(operation: CalendarOperation, is_static: impl Fn(&str) -> bool) -> Self {
        match operation {
            CalendarOperation::Create(event) => Self::Create {
                static_anchor: is_static(&event.task_id),
                event: event.into(),
            },
            CalendarOperation::Update(event) => Self::Update {
                static_anchor: is_static(&event.task_id),
                event: event.into(),
            },
            CalendarOperation::Delete {
                external_id,
                summary,
            } => Self::Delete {
                external_id,
                summary,
            },
        }
    }
}

/// Persisted preview of the current Calendar, diffed against the last applied
/// event set. Preview changes projection records only; it makes no client calls.
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarProjectionPreviewResponse {
    pub schema_version: String,
    pub preview_id: String,
    pub plan_id: Option<String>,
    pub stale: bool,
    pub events: Vec<CalendarEventBody>,
    /// Current Dynamic Plan placements matching the applied snapshot; excludes Static commitments and retained completed history.
    pub matching_placements: usize,
    pub operations: Vec<CalendarOperationBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct CalendarPreviewQuery {
    /// Mirror the existing projection policy input; true denies external export.
    #[serde(default)]
    pub no_external_export: bool,
}

#[utoipa::path(
    get,
    path = "/projection/calendar/preview",
    params(CalendarPreviewQuery),
    responses((status = 200, body = CalendarProjectionPreviewResponse))
)]
pub async fn preview(
    State(state): State<AppState>,
    Query(query): Query<CalendarPreviewQuery>,
) -> Result<Json<CalendarProjectionPreviewResponse>> {
    Ok(Json(calendar_apply::preview(&state, query.no_external_export).await?))
}

pub const CALENDAR_PROJECTION_APPROVAL_SCHEMA_VERSION: &str = "ubu.orchestrator.calendar_projection_approval.v1";

#[derive(Debug, Deserialize, ToSchema)]
pub struct CalendarProjectionApproveRequest {
    pub schema_version: Option<String>,
    pub preview_id: String,
    pub authority_source: crate::api::projection::AuthoritySourceBody,
    pub export_mode: crate::services::calendar_client::CalendarExportMode,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarOperationResultBody {
    pub operation_id: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarProjectionResultResponse {
    pub schema_version: String,
    pub preview_id: String,
    pub status: String,
    pub applied_events: Vec<CalendarEventBody>,
    pub operation_results: Vec<CalendarOperationResultBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

impl From<calendar_apply::StoredCalendarResult> for CalendarProjectionResultResponse {
    fn from(result: calendar_apply::StoredCalendarResult) -> Self {
        use ubu_core::projection::{OperationResultStatus, ProjectionResultStatus};
        Self {
            schema_version: result.schema_version, preview_id: result.preview_id,
            status: match result.status {ProjectionResultStatus::Applied=>"applied",ProjectionResultStatus::Partial=>"partial",ProjectionResultStatus::Failed=>"failed"}.into(),
            applied_events: result.applied_events.into_iter().map(Into::into).collect(),
            operation_results: result.operation_results.into_iter().map(|operation|CalendarOperationResultBody {
                operation_id:operation.operation_id,
                status:match operation.status {OperationResultStatus::Applied=>"applied",OperationResultStatus::Skipped=>"skipped",OperationResultStatus::Failed=>"failed"}.into(),
                message:operation.message,
            }).collect(),
            diagnostics:result.diagnostics,
        }
    }
}

#[utoipa::path(
    post,
    path = "/projection/calendar/approve",
    request_body = CalendarProjectionApproveRequest,
    responses((status = 200, body = CalendarProjectionResultResponse),
        (status = 409, description = "The applied set changed since this preview"),
        (status = 403, description = "Live Calendar export is not enabled for this process"),
        (status = 503, description = "Google credential paths are not configured"))
)]
pub async fn approve(
    State(state): State<AppState>,
    Json(request): Json<CalendarProjectionApproveRequest>,
) -> Result<Json<CalendarProjectionResultResponse>> {
    match request.schema_version.as_deref() {
        Some(CALENDAR_PROJECTION_APPROVAL_SCHEMA_VERSION) => {},
        Some(other) => return Err(crate::errors::AppError::bad_request_diagnostic("unknown_schema_version",format!("unsupported schema_version `{other}`"))),
        None => return Err(crate::errors::AppError::bad_request_diagnostic("missing_schema_version","schema_version is required")),
    }
    Ok(Json(calendar_apply::approve(&state,&request.preview_id,request.authority_source.into(),request.export_mode).await?.into()))
}
