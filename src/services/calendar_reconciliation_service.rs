//! Calendar reconciliation I/O; classification and repair remain pure.
use std::collections::BTreeSet;

use crate::{errors::Result, services::calendar_projection::external_id};

/// All active Tasks contribute evidence, whether currently scheduled or not.
/// A derivable id does not confer ownership; only an applied record does that.
pub async fn known_external_ids(pool: &sqlx::SqlitePool) -> Result<BTreeSet<String>> {
    Ok(ubu_store::queries::query_active_tasks(pool)
        .await?
        .into_iter()
        .filter_map(|task| external_id(&task.id))
        .collect())
}

use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use ubu_core::{projection::ProjectionResultStatus, ObjectType, UbuId};

use super::{
    calendar_apply::{self, StoredCalendarResult, CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION},
    calendar_client::{CalendarApi, CalendarExportMode, RecordingCalendarApi},
    calendar_google::GoogleCalendarApi,
    calendar_projection::DesiredEvent,
    calendar_reconcile::{self, CalendarConflict},
};
use crate::{
    api::calendar_reconciliation::{
        CalendarReconcileResponse, CalendarRepairResponse, CALENDAR_RECONCILIATION_SCHEMA_VERSION,
        CALENDAR_REPAIR_SCHEMA_VERSION,
    },
    errors::AppError,
    state::AppState,
};

#[derive(Deserialize)]
struct RecordedObservation {
    schema_version: String,
    preview_id: String,
    applied_events: Vec<DesiredEvent>,
    observed_events: Vec<DesiredEvent>,
    conflicts: Vec<CalendarConflict>,
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

pub async fn reconcile(
    state: &AppState,
    mode: CalendarExportMode,
) -> Result<CalendarReconcileResponse> {
    mode.ensure_available(state)?;
    let _guard = state.inner().calendar_projection_lock.lock().await;
    let pool = state.inner().store.pool();
    let applied = calendar_apply::last_applied_events(pool).await?;
    // Match last_applied_events' schema/status/order exactly for source links.
    let previous = sqlx::query("SELECT id, preview_id FROM projection_results WHERE status IN ('applied','partial') AND json_extract(payload_json,'$.schema_version')=? ORDER BY rowid DESC LIMIT 1")
        .bind(CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION).fetch_optional(pool).await.map_err(internal)?;
    let result_id: Option<String> = previous
        .as_ref()
        .map(|row| row.try_get("id"))
        .transpose()
        .map_err(internal)?;
    // Before the first apply there is no source preview. Reserve a correctly
    // typed bookkeeping id for the existing NOT NULL preview_id columns; this
    // does not create a preview, invent ownership, or claim a prior delivery.
    let preview_id: String = previous
        .as_ref()
        .map(|row| row.try_get("preview_id"))
        .transpose()
        .map_err(internal)?
        .unwrap_or_else(|| UbuId::new(ObjectType::ProjectionPreview).to_string());
    let google = if mode == CalendarExportMode::Live {
        Some(Arc::new(
            GoogleCalendarApi::new(&state.inner().config).map_err(internal)?,
        ))
    } else {
        None
    };
    let client: Arc<dyn CalendarApi> = match &google {
        Some(client) => client.clone(),
        None => state
            .calendar_api()
            .unwrap_or_else(|| Arc::new(RecordingCalendarApi::with_events(applied.clone()))),
    };
    let observed = client.list_events().await.map_err(AppError::Upstream)?;
    let diagnostics = match &google {
        Some(client) => client.take_diagnostics().await,
        None => Vec::new(),
    };
    let known = known_external_ids(pool).await?;
    let conflicts = calendar_reconcile::classify(&applied, &observed, &known);
    let status = if conflicts.is_empty() {
        "matched"
    } else if conflicts
        .iter()
        .any(|conflict| matches!(conflict.conflict_type.as_str(), "drifted" | "missing"))
    {
        "drifted"
    } else {
        "observed"
    };
    let reconciliation_id = UbuId::new(ObjectType::Snapshot).to_string();
    let now = state.planning_now().to_string();
    let payload = json!({
        "schema_version": CALENDAR_RECONCILIATION_SCHEMA_VERSION,
        "reconciliation_id": reconciliation_id,
        "preview_id": preview_id,
        "result_id": result_id,
        "status": status,
        "applied_events": applied,
        "observed_events": observed,
        "known_external_ids": known,
        "conflicts": conflicts,
        "diagnostics": diagnostics,
        "repaired": false,
    });
    sqlx::query("INSERT INTO projection_reconciliations (id, preview_id, result_id, status, payload_json, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&reconciliation_id).bind(&preview_id).bind(result_id.as_deref().unwrap_or(""))
        .bind(status).bind(payload.to_string()).bind(now).execute(pool).await.map_err(internal)?;
    Ok(CalendarReconcileResponse {
        schema_version: CALENDAR_RECONCILIATION_SCHEMA_VERSION.into(),
        reconciliation_id,
        status: status.into(),
        conflicts,
        diagnostics,
    })
}

/// Repair never constructs a Calendar client, re-reads Google, or invokes the
/// export gate. It trusts exactly the observation and ownership recorded above.
pub async fn repair(state: &AppState, reconciliation_id: &str) -> Result<CalendarRepairResponse> {
    let _guard = state.inner().calendar_projection_lock.lock().await;
    let pool = state.inner().store.pool();
    let mut transaction = pool.begin().await.map_err(internal)?;
    let row = sqlx::query("SELECT status, payload_json FROM projection_reconciliations WHERE id=?")
        .bind(reconciliation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Calendar reconciliation not found".into()))?;
    let raw: String = row.try_get("payload_json").map_err(internal)?;
    let mut payload: serde_json::Value = serde_json::from_str(&raw).map_err(internal)?;
    if payload["schema_version"] != CALENDAR_RECONCILIATION_SCHEMA_VERSION {
        return Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            "stored record is not a supported Calendar reconciliation",
        ));
    }
    let status: String = row.try_get("status").map_err(internal)?;
    if status == "repaired" {
        return Err(AppError::conflict_diagnostic(
            "calendar_reconciliation_already_repaired",
            "This Calendar reconciliation has already been repaired; request a new reconciliation",
        ));
    }
    let observation: RecordedObservation =
        serde_json::from_value(payload.clone()).map_err(internal)?;
    debug_assert_eq!(
        observation.schema_version,
        CALENDAR_RECONCILIATION_SCHEMA_VERSION
    );
    let repaired =
        calendar_reconcile::repair(&observation.applied_events, &observation.observed_events);
    let updated_events = repaired
        .iter()
        .filter(|current| {
            observation
                .applied_events
                .iter()
                .any(|old| old.external_id == current.external_id && old != *current)
        })
        .count();
    let response = CalendarRepairResponse {
        schema_version: CALENDAR_REPAIR_SCHEMA_VERSION.into(),
        reconciliation_id: reconciliation_id.into(),
        dropped_events: observation.applied_events.len() - repaired.len(),
        updated_events,
        applied_event_count: repaired.len(),
        remaining_conflicts: observation
            .conflicts
            .into_iter()
            .filter(|conflict| matches!(conflict.conflict_type.as_str(), "foreign" | "unrecorded"))
            .collect(),
    };
    let result = StoredCalendarResult {
        schema_version: CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION.into(),
        preview_id: observation.preview_id,
        status: ProjectionResultStatus::Applied,
        applied_events: repaired,
        operation_results: Vec::new(),
        diagnostics: Vec::new(),
    };
    let result_id = UbuId::new(ObjectType::Snapshot).to_string();
    let now = state.planning_now().to_string();
    let mut result_payload = serde_json::to_value(&result).map_err(internal)?;
    result_payload["reconciliation_id"] = reconciliation_id.into();
    result_payload["repair"] = true.into();
    // Same projection-result contract as calendar_apply::persist_result, with
    // its insert and the one-shot marker committed atomically. No schema change.
    sqlx::query("INSERT INTO projection_results (id, preview_id, status, payload_json, created_at) VALUES (?, ?, 'applied', ?, ?)")
        .bind(&result_id).bind(&result.preview_id).bind(result_payload.to_string()).bind(&now)
        .execute(&mut *transaction).await.map_err(internal)?;
    payload["repaired"] = true.into();
    payload["repaired_result_id"] = result_id.into();
    payload["repaired_at"] = now.into();
    sqlx::query(
        "UPDATE projection_reconciliations SET status='repaired', payload_json=? WHERE id=?",
    )
    .bind(payload.to_string())
    .bind(reconciliation_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal)?;
    transaction.commit().await.map_err(internal)?;
    Ok(response)
}
