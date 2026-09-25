//! Calendar-specific reconciliation contracts; no GitHub label-shaped fields.
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    services::{
        calendar_client::CalendarExportMode, calendar_reconcile::CalendarConflict,
        calendar_reconciliation_service,
    },
    state::AppState,
};

pub const CALENDAR_RECONCILIATION_SCHEMA_VERSION: &str =
    "ubu.orchestrator.calendar_reconciliation.v1";
pub const CALENDAR_REPAIR_SCHEMA_VERSION: &str = "ubu.orchestrator.calendar_repair.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarReconcileRequest {
    pub schema_version: Option<String>,
    pub export_mode: CalendarExportMode,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarReconcileResponse {
    pub schema_version: String,
    pub reconciliation_id: String,
    pub status: String,
    pub conflicts: Vec<CalendarConflict>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarRepairResponse {
    pub schema_version: String,
    pub reconciliation_id: String,
    pub dropped_events: usize,
    pub updated_events: usize,
    pub applied_event_count: usize,
    pub remaining_conflicts: Vec<CalendarConflict>,
}

#[utoipa::path(
    post,
    path = "/projection/calendar/reconcile",
    request_body = CalendarReconcileRequest,
    responses((status = 200, body = CalendarReconcileResponse),
        (status = 400, description = "Missing or unsupported schema"),
        (status = 403, description = "Live Calendar is not enabled for this process"),
        (status = 503, description = "Google credential paths are not configured"),
        (status = 502, description = "Calendar observation failed"))
)]
pub async fn reconcile(
    State(state): State<AppState>,
    Json(request): Json<CalendarReconcileRequest>,
) -> Result<Json<CalendarReconcileResponse>> {
    match request.schema_version.as_deref() {
        Some(CALENDAR_RECONCILIATION_SCHEMA_VERSION) => {}
        Some(_) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                "unsupported Calendar reconciliation schema_version",
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
        calendar_reconciliation_service::reconcile(&state, request.export_mode).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/projection/calendar/reconcile/{reconciliation_id}/repair",
    params(("reconciliation_id" = String, Path)),
    responses((status = 200, body = CalendarRepairResponse),
        (status = 400, description = "Record is not a supported Calendar reconciliation"),
        (status = 404, description = "Reconciliation not found"),
        (status = 409, description = "Calendar reconciliation has already been repaired"))
)]
pub async fn repair(
    State(state): State<AppState>,
    Path(reconciliation_id): Path<String>,
) -> Result<Json<CalendarRepairResponse>> {
    Ok(Json(
        calendar_reconciliation_service::repair(&state, &reconciliation_id).await?,
    ))
}
