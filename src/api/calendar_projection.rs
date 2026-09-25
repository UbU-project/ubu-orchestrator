use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    api::planning::DiagnosticBody,
    errors::Result,
    services::{
        calendar_projection::{self, CalendarOperation, DesiredEvent},
        planning_service,
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

/// Read-only view of the current Calendar. Preview stores nothing. Operations
/// are diff(desired, &[]): all creates until P1B-29 persists the applied events.
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarProjectionPreviewResponse {
    pub schema_version: String,
    pub plan_id: Option<String>,
    pub stale: bool,
    pub events: Vec<CalendarEventBody>,
    pub operations: Vec<CalendarOperationBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[utoipa::path(
    get,
    path = "/projection/calendar/preview",
    responses((status = 200, body = CalendarProjectionPreviewResponse))
)]
pub async fn preview(
    State(state): State<AppState>,
) -> Result<Json<CalendarProjectionPreviewResponse>> {
    let calendar = planning_service::current_calendar(state.clone()).await?;
    let maps = planning_service::calendar_reminder_maps(state.inner().store.pool()).await?;
    let desired = calendar_projection::desired_events(
        &calendar.steps,
        &maps.reminders_by_objective,
        &maps.objective_of_task,
    );
    let operations = calendar_projection::diff(&desired, &[])
        .into_iter()
        .map(Into::into)
        .collect();
    let diagnostics = calendar
        .steps
        .iter()
        .filter(|step| calendar_projection::external_id(&step.task_id).is_none())
        .map(|step| DiagnosticBody {
            code: "calendar_event_id_unmappable".into(),
            message: format!(
                "Task `{}` cannot produce a valid Calendar event id; step skipped",
                step.task_id
            ),
        })
        .collect();
    Ok(Json(CalendarProjectionPreviewResponse {
        schema_version: CALENDAR_PROJECTION_PREVIEW_SCHEMA_VERSION.into(),
        plan_id: calendar.plan_id,
        stale: calendar.stale,
        events: desired.into_iter().map(Into::into).collect(),
        operations,
        diagnostics,
    }))
}
