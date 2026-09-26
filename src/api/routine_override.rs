//! Explicit per-date routine triage; no Calendar transport is involved.
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    services::routine_override,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use ubu_core::UbuTimestamp;
use utoipa::ToSchema;

pub const SCHEMA_VERSION: &str = "ubu.orchestrator.routine_override.v1";
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OverrideRequest {
    pub schema_version: Option<String>,
    pub start: String,
    pub end: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct OverrideResponse {
    pub schema_version: String,
    pub objective_id: String,
    pub local_date: String,
    pub overridden: bool,
    pub diagnostics: Vec<DiagnosticBody>,
}
impl From<routine_override::OverrideOutcome> for OverrideResponse {
    fn from(outcome: routine_override::OverrideOutcome) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.into(),
            objective_id: outcome.objective_id,
            local_date: outcome.local_date,
            overridden: outcome.overridden,
            diagnostics: outcome.diagnostics,
        }
    }
}
#[utoipa::path(put, path="/routine/{objective_id}/override/{local_date}",
    params(("objective_id"=String, Path), ("local_date"=String, Path)),
    request_body=OverrideRequest,
    responses((status=200, body=OverrideResponse), (status=400, description="Invalid schema, window, routine or recurrence date"), (status=409, description="Concurrent canonical edit")))]
pub async fn set(
    State(state): State<AppState>,
    Path((objective_id, local_date)): Path<(String, String)>,
    Json(request): Json<OverrideRequest>,
) -> Result<Json<OverrideResponse>> {
    match request.schema_version.as_deref() {
        Some(SCHEMA_VERSION) => {}
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
        Some(_) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                "Unsupported routine override schema_version",
            ))
        }
    }
    let parse = |value: &str| {
        UbuTimestamp::parse(value).map_err(|error| {
            AppError::bad_request_diagnostic("routine_override_invalid_window", error.to_string())
        })
    };
    let outcome = routine_override::change(
        &state,
        &objective_id,
        &local_date,
        Some((parse(&request.start)?, parse(&request.end)?)),
        None,
    )
    .await?;
    Ok(Json(outcome.into()))
}
#[utoipa::path(delete, path="/routine/{objective_id}/override/{local_date}",
    params(("objective_id"=String, Path), ("local_date"=String, Path)),
    responses((status=200, body=OverrideResponse), (status=400, description="Invalid routine, recurrence date or nominal restoration"), (status=409, description="Concurrent canonical edit")))]
pub async fn clear(
    State(state): State<AppState>,
    Path((objective_id, local_date)): Path<(String, String)>,
) -> Result<Json<OverrideResponse>> {
    Ok(Json(
        routine_override::change(&state, &objective_id, &local_date, None, None)
            .await?
            .into(),
    ))
}
