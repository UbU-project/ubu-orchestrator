//! Persisted Calendar projection state, separate from the GitHub projection path.
use serde::{Deserialize, Serialize};
use sqlx::Row;
use ubu_core::{
    projection::{OperationResult, ProjectionResultStatus},
    Legitimization, ObjectType, PolicySummary, UbuId, UbuTimestamp,
};
use ubu_store::{
    models::projection_record::{NewProjectionPreviewRecord, NewProjectionResultRecord},
    queries,
};

use super::{
    calendar_projection::{self, CalendarOperation, DesiredEvent},
    planning_service,
};
use crate::{
    api::{
        calendar_projection::{
            CalendarProjectionPreviewResponse, CALENDAR_PROJECTION_PREVIEW_SCHEMA_VERSION,
        },
        planning::DiagnosticBody,
    },
    errors::{AppError, Result},
    state::AppState,
};

pub const CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION: &str =
    "ubu.orchestrator.calendar_projection_result.v1";

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredCalendarPreview {
    pub schema_version: String,
    pub preview_id: String,
    pub plan_id: Option<String>,
    pub stale: bool,
    pub desired_events: Vec<DesiredEvent>,
    pub existing_events: Vec<DesiredEvent>,
    pub operations: Vec<CalendarOperation>,
    pub policy_summary: Option<PolicySummary>,
}

#[derive(Debug, Serialize)]
pub struct StoredCalendarResult {
    pub schema_version: String,
    pub preview_id: String,
    pub status: ProjectionResultStatus,
    pub applied_events: Vec<DesiredEvent>,
    pub operation_results: Vec<OperationResult>,
    pub diagnostics: Vec<DiagnosticBody>,
}

/// A partial result is successful for the operations that landed. Its full
/// applied-set snapshot must supersede the previous one, or retries duplicate
/// those successful operations. Failed batches have no landed changes.
pub async fn last_applied_events(pool: &sqlx::SqlitePool) -> Result<Vec<DesiredEvent>> {
    #[derive(Deserialize)]
    struct AppliedSnapshot {
        applied_events: Vec<DesiredEvent>,
    }
    let raw:Option<String>=sqlx::query_scalar("SELECT payload_json FROM projection_results WHERE status IN ('applied','partial') AND json_extract(payload_json,'$.schema_version')=? ORDER BY rowid DESC LIMIT 1")
        .bind(CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION).fetch_optional(pool).await.map_err(internal)?;
    raw.map(|raw| {
        serde_json::from_str::<AppliedSnapshot>(&raw)
            .map(|result| result.applied_events)
            .map_err(internal)
    })
    .unwrap_or_else(|| Ok(Vec::new()))
}

pub fn resolved_policy_summary(no_external_export: bool, now: UbuTimestamp) -> PolicySummary {
    PolicySummary {
        legitimization: if no_external_export {
            Legitimization::Rejected
        } else {
            Legitimization::Accepted
        },
        adjudication_reasons: vec![if no_external_export {
            "effective compartment policy forbids external export"
        } else {
            "Calendar projection is allowed for automation worker export"
        }
        .into()],
        local_only: Some(false),
        no_cloud_llm: Some(false),
        no_external_export: Some(no_external_export),
        checked_at: now,
    }
}

pub async fn preview(
    state: &AppState,
    no_external_export: bool,
) -> Result<CalendarProjectionPreviewResponse> {
    let _guard = state.inner().calendar_projection_lock.lock().await;
    let calendar = planning_service::current_calendar(state.clone()).await?;
    let maps = planning_service::calendar_reminder_maps(state.inner().store.pool()).await?;
    let desired = calendar_projection::desired_events(
        &calendar.steps,
        &maps.reminders_by_objective,
        &maps.objective_of_task,
    );
    let existing = last_applied_events(state.inner().store.pool()).await?;
    let operations = calendar_projection::diff(&desired, &existing);
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
    let now = state.planning_now();
    let stored = StoredCalendarPreview {
        schema_version: CALENDAR_PROJECTION_PREVIEW_SCHEMA_VERSION.into(),
        preview_id: UbuId::new(ObjectType::ProjectionPreview).to_string(),
        plan_id: calendar.plan_id,
        stale: calendar.stale,
        desired_events: desired,
        existing_events: existing,
        operations,
        policy_summary: Some(resolved_policy_summary(no_external_export, now)),
    };
    queries::store_projection_preview(
        state.inner().store.pool(),
        NewProjectionPreviewRecord {
            id: stored.preview_id.clone(),
            request_id: stored.preview_id.clone(),
            status: "pending".into(),
            payload: serde_json::to_value(&stored).map_err(internal)?,
            created_at: now.to_string(),
        },
    )
    .await?;
    Ok(CalendarProjectionPreviewResponse {
        schema_version: stored.schema_version,
        preview_id: stored.preview_id,
        plan_id: stored.plan_id,
        stale: stored.stale,
        events: stored.desired_events.into_iter().map(Into::into).collect(),
        operations: stored.operations.into_iter().map(Into::into).collect(),
        diagnostics,
    })
}

pub async fn load_preview(
    pool: &sqlx::SqlitePool,
    preview_id: &str,
) -> Result<StoredCalendarPreview> {
    let row = sqlx::query("SELECT payload_json FROM projection_previews WHERE id=?")
        .bind(preview_id)
        .fetch_optional(pool)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Calendar projection preview not found".into()))?;
    let raw: String = row.try_get("payload_json").map_err(internal)?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(internal)?;
    if value["schema_version"] != CALENDAR_PROJECTION_PREVIEW_SCHEMA_VERSION {
        return Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            "stored preview is not a supported Calendar projection preview",
        ));
    }
    serde_json::from_value(value).map_err(internal)
}

pub async fn persist_result(
    pool: &sqlx::SqlitePool,
    result: &StoredCalendarResult,
    now: UbuTimestamp,
) -> Result<()> {
    let status = match result.status {
        ProjectionResultStatus::Applied => "applied",
        ProjectionResultStatus::Partial => "partial",
        ProjectionResultStatus::Failed => "failed",
    };
    queries::store_projection_result(
        pool,
        NewProjectionResultRecord {
            id: UbuId::new(ObjectType::Snapshot).to_string(),
            preview_id: result.preview_id.clone(),
            status: status.into(),
            payload: serde_json::to_value(result).map_err(internal)?,
            created_at: now.to_string(),
        },
    )
    .await?;
    Ok(())
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// Apply exactly the stored preview under the same single-Device lock used by
/// preview. The client never supplies the diff's existing side.
pub async fn approve(
    state: &AppState,
    preview_id: &str,
    authority: ubu_core::AuthoritySource,
    mode: super::calendar_client::CalendarExportMode,
) -> Result<StoredCalendarResult> {
    use super::calendar_client::{CalendarApi, RecordingCalendarApi};
    use std::{collections::BTreeMap, sync::Arc};
    use ubu_core::projection::OperationResultStatus;

    mode.ensure_available()?;
    let _guard = state.inner().calendar_projection_lock.lock().await;
    let pool = state.inner().store.pool();
    let stored = load_preview(pool, preview_id).await?;
    let existing = last_applied_events(pool).await?;
    if existing != stored.existing_events {
        return Err(AppError::conflict_diagnostic(
            "calendar_projection_conflict",
            "The applied Calendar event set changed after this preview; request a new preview",
        ));
    }
    let client: Arc<dyn CalendarApi> = state
        .calendar_api()
        .unwrap_or_else(|| Arc::new(RecordingCalendarApi::with_events(existing.clone())));
    let mut landed: BTreeMap<_, _> = existing
        .into_iter()
        .map(|event| (event.external_id.clone(), event))
        .collect();
    let mut operation_results = Vec::new();
    let mut diagnostics = Vec::new();
    for operation in &stored.operations {
        let core = lower_operation(operation)?;
        let gate = gate_export_operation(state, &core, stored.policy_summary.as_ref(), authority);
        append_boundary_log(state, preview_id, &core, &gate.decision.log_payload).await?;
        let Some(permit) = gate.permit() else {
            let message = gate.decision.adjudication_reasons.join(" ");
            diagnostics.push(DiagnosticBody {
                code: "calendar_export_rejected".into(),
                message: message.clone(),
            });
            operation_results.push(OperationResult {
                operation_id: core.operation_id,
                status: OperationResultStatus::Skipped,
                message: Some(message),
            });
            continue;
        };
        if permit.operation_id() != core.operation_id
            || permit.authority_source() != ubu_core::AuthoritySource::AutomationWorker
        {
            return Err(AppError::Internal(
                "Calendar export permit does not match the operation".into(),
            ));
        }
        let applied = match operation {
            CalendarOperation::Create(event) => client.insert_event(event).await,
            CalendarOperation::Update(event) => client.patch_event(event).await,
            CalendarOperation::Delete { external_id, .. } => client.delete_event(external_id).await,
        };
        match applied {
            Ok(()) => {
                match operation {
                    CalendarOperation::Create(event) | CalendarOperation::Update(event) => {
                        landed.insert(event.external_id.clone(), event.clone());
                    }
                    CalendarOperation::Delete { external_id, .. } => {
                        landed.remove(external_id);
                    }
                }
                operation_results.push(OperationResult {
                    operation_id: core.operation_id,
                    status: OperationResultStatus::Applied,
                    message: None,
                });
            }
            Err(message) => {
                diagnostics.push(DiagnosticBody {
                    code: "calendar_operation_failed".into(),
                    message: format!("{}: {message}", core.operation_id),
                });
                operation_results.push(OperationResult {
                    operation_id: core.operation_id,
                    status: OperationResultStatus::Failed,
                    message: Some(message),
                });
            }
        }
    }
    let applied = operation_results
        .iter()
        .filter(|result| result.status == OperationResultStatus::Applied)
        .count();
    let status = if applied == operation_results.len() {
        ProjectionResultStatus::Applied
    } else if applied > 0 {
        ProjectionResultStatus::Partial
    } else {
        ProjectionResultStatus::Failed
    };
    let result = StoredCalendarResult {
        schema_version: CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION.into(),
        preview_id: preview_id.into(),
        status,
        applied_events: landed.into_values().collect(),
        operation_results,
        diagnostics,
    };
    persist_result(pool, &result, state.planning_now()).await?;
    Ok(result)
}

pub fn lower_operation(
    operation: &CalendarOperation,
) -> Result<ubu_core::projection::ProjectionOperation> {
    use ubu_core::{
        projection::{ProjectionOperation, ProjectionOperationKind},
        SourceRef,
    };
    let (kind, verb, id, summary, payload) = match operation {
        CalendarOperation::Create(event) => (
            ProjectionOperationKind::Create,
            "create",
            &event.external_id,
            &event.summary,
            serde_json::to_value(event).map_err(internal)?,
        ),
        CalendarOperation::Update(event) => (
            ProjectionOperationKind::Update,
            "update",
            &event.external_id,
            &event.summary,
            serde_json::to_value(event).map_err(internal)?,
        ),
        CalendarOperation::Delete {
            external_id,
            summary,
        } => (
            ProjectionOperationKind::Delete,
            "delete",
            external_id,
            summary,
            serde_json::json!({"external_id":external_id,"summary":summary}),
        ),
    };
    Ok(ProjectionOperation {
        operation_id: format!("calendar-{verb}-{id}"),
        kind,
        target: SourceRef {
            source_kind: "google_calendar".into(),
            source_id: id.clone(),
            url: None,
        },
        summary: summary.clone(),
        payload: Some(payload),
    })
}

fn gate_export_operation(
    state: &AppState,
    operation: &ubu_core::projection::ProjectionOperation,
    policy: Option<&PolicySummary>,
    authority: ubu_core::AuthoritySource,
) -> ubu_core::projection::ExportGateDecision {
    use ubu_core::{
        projection::{ExportProjectionContext, Legitimizer},
        ObjectRef, Provenance,
    };
    let now = state.planning_now();
    let compartment = ObjectRef {
        id: UbuId::new(ObjectType::Compartment),
        object_type: ObjectType::Compartment,
    };
    let actor = ObjectRef {
        id: state.actor_identity_id().clone(),
        object_type: ObjectType::Identity,
    };
    let provenance = Provenance {
        created_at: now,
        created_by: None,
        authority_source: authority,
        source: Some(operation.target.clone()),
        source_refs: None,
    };
    Legitimizer::gate_export_projection(ExportProjectionContext {
        operation,
        effective_policy: policy,
        compartment_ref: &compartment,
        actor_identity_ref: &actor,
        authority_source: authority,
        effective_time: now,
        provenance: &provenance,
    })
}

async fn append_boundary_log(
    state: &AppState,
    preview_id: &str,
    operation: &ubu_core::projection::ProjectionOperation,
    payload: &ubu_core::core::CompartmentBoundaryDecidedPayload,
) -> Result<()> {
    let envelope = state.envelope_for(
        Default::default(),
        payload.authority_source,
        payload.effective_time,
    )?;
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::log_record::NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "compartment_boundary_decided".into(),
            object_refs: serde_json::json!([preview_id, operation.operation_id]),
            payload: serde_json::to_value(payload).map_err(internal)?,
            provenance: serde_json::to_value(&payload.provenance).map_err(internal)?,
            created_at: payload.effective_time.to_string(),
        },
    )
    .await?;
    Ok(())
}
