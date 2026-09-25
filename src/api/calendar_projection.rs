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
    },
    Update {
        event: CalendarEventBody,
    },
    Delete {
        external_id: String,
        summary: String,
    },
}

impl From<CalendarOperation> for CalendarOperationBody {
    fn from(operation: CalendarOperation) -> Self {
        match operation {
            CalendarOperation::Create(event) => Self::Create {
                event: event.into(),
            },
            CalendarOperation::Update(event) => Self::Update {
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
