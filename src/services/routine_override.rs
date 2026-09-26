//! Canonical, per-date placement decisions shared by Calendar gestures and triage.
use super::routine_instantiation::{instantiate, matches, RoutineDefinition};
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    state::AppState,
};
use chrono::{LocalResult, NaiveDate, TimeZone};
use chrono_tz::Tz;
use serde_json::{json, Value};
use ubu_core::{
    core::{Objective, RoutineOccurrenceOverride},
    AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef,
};
use ubu_store::{
    models::{log_record::NewLogRecord, object_record::NewObjectRecord},
    queries,
};

pub struct CalendarSource<'a> {
    pub task_id: &'a str,
    pub external_id: &'a str,
}
pub struct OverrideOutcome {
    pub objective_id: String,
    pub local_date: String,
    pub overridden: bool,
    pub diagnostics: Vec<DiagnosticBody>,
}
fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(e.to_string())
}

fn day_bounds(timezone: &str, day: &str) -> Result<(u64, u64)> {
    ubu_core::core::validate_local_date(day).map_err(|e| {
        AppError::bad_request_diagnostic("routine_override_invalid_date", e.to_string())
    })?;
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(internal)?;
    let zone: Tz = timezone.parse().map_err(|_| {
        AppError::bad_request_diagnostic(
            "routine_timezone_unknown",
            format!("Unknown routine timezone `{timezone}`"),
        )
    })?;
    let midnight = |date: NaiveDate| {
        let time = date.and_hms_opt(0, 0, 0).unwrap();
        let instant = match zone.from_local_datetime(&time) {
            LocalResult::Single(t) => t,
            LocalResult::Ambiguous(a, b) => a.min(b),
            LocalResult::None => {
                return Err(AppError::bad_request_diagnostic(
                    "routine_override_invalid_date",
                    "The local date has no resolvable midnight",
                ))
            }
        };
        u64::try_from(instant.timestamp()).map_err(|_| {
            AppError::bad_request_diagnostic(
                "routine_override_invalid_date",
                "The local date precedes supported planning time",
            )
        })
    };
    let next = date.succ_opt().ok_or_else(|| {
        AppError::bad_request_diagnostic(
            "routine_override_invalid_date",
            "Local date is out of range",
        )
    })?;
    Ok((midnight(date)?, midnight(next)?))
}
fn guard(state: &AppState, bounds: (u64, u64), start: u64, end: u64) -> Result<()> {
    if start >= end {
        return Err(AppError::bad_request_diagnostic(
            "routine_override_invalid_window",
            "Override window must have a strictly positive span",
        ));
    }
    let horizon = state.inner().planning_horizon_seconds;
    if start < bounds.0.saturating_sub(horizon) || end > bounds.1.saturating_add(horizon) {
        return Err(AppError::bad_request_diagnostic("routine_override_window_too_far", "Override window must stay within its local day plus one configured horizon length on either side"));
    }
    Ok(())
}

/// The caller may hold the Calendar projection lock; imports/materialization
/// precede action locking, matching the existing canonical admission paths.
pub async fn change(
    state: &AppState,
    objective_id: &str,
    local_date: &str,
    window: Option<(UbuTimestamp, UbuTimestamp)>,
    source: Option<CalendarSource<'_>>,
) -> Result<OverrideOutcome> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _materialize = state.inner().routine_materialization_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    let pool = state.inner().store.pool();
    let row = queries::get_current_state(pool, objective_id)
        .await?
        .filter(|row| row.object_type == "Objective" && row.status == "active")
        .ok_or_else(|| {
            AppError::bad_request_diagnostic(
                "routine_override_unknown_routine",
                format!("Active routine `{objective_id}` was not found"),
            )
        })?;
    let original: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
    let mut objective: Objective = serde_json::from_value(original.clone()).map_err(internal)?;
    let schedule = objective.recurrence.as_mut().ok_or_else(|| {
        AppError::bad_request_diagnostic(
            "routine_override_unknown_routine",
            "Objective has no recurrence schedule",
        )
    })?;
    let template = objective.routine_instance_template.clone().ok_or_else(|| {
        AppError::bad_request_diagnostic(
            "routine_override_unknown_routine",
            "Objective has no routine template",
        )
    })?;
    let bounds = day_bounds(&schedule.timezone, local_date)?;
    let date = NaiveDate::parse_from_str(local_date, "%Y-%m-%d").map_err(internal)?;
    let absent = || {
        AppError::bad_request_diagnostic(
            "routine_override_no_occurrence",
            format!("Routine `{objective_id}` does not occur on {local_date}"),
        )
    };
    if !matches(schedule, date) {
        return Err(absent());
    }
    if let Some(source) = &source {
        let task = queries::get_current_state(pool, source.task_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Task `{}` disappeared", source.task_id)))?;
        let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
        if task.status != "active" {
            return Err(AppError::bad_request_diagnostic(
                "calendar_gesture_on_inactive_task",
                format!(
                    "Task `{}` is {}; override ignored",
                    source.task_id, task.status
                ),
            ));
        }
        if payload["occurrence"]["routine_objective_id"] != objective_id
            || payload["occurrence"]["local_date"] != local_date
        {
            return Err(AppError::conflict_diagnostic(
                "routine_override_occurrence_changed",
                "Calendar occurrence identity changed before admission",
            ));
        }
    }
    schedule
        .overrides
        .retain(|entry| entry.local_date != local_date);
    if let Some((start, end)) = window {
        let entry = RoutineOccurrenceOverride {
            local_date: local_date.into(),
            start,
            end,
        };
        entry.validate().map_err(|e| {
            AppError::bad_request_diagnostic("routine_override_invalid_window", e.to_string())
        })?;
        let seconds = |value: UbuTimestamp| {
            u64::try_from(value.inner().unix_timestamp()).map_err(|_| {
                AppError::bad_request_diagnostic(
                    "routine_override_invalid_window",
                    "Window precedes supported planning time",
                )
            })
        };
        guard(state, bounds, seconds(start)?, seconds(end)?)?;
        schedule.overrides.push(entry);
        schedule
            .overrides
            .sort_by(|a, b| a.local_date.cmp(&b.local_date));
    }
    let mut definitions = super::routine_service::live_definitions(pool)
        .await?
        .definitions;
    definitions.retain(|definition| definition.objective_id.as_str() != objective_id);
    definitions.push(RoutineDefinition {
        objective_id: objective.id.clone(),
        schedule: schedule.clone(),
        template,
    });
    let derived = instantiate(&definitions, bounds.0, bounds.1.saturating_sub(1));
    let occurrence = derived
        .occurrences
        .iter()
        .find(|o| o.objective_id.as_str() == objective_id && o.local_date == local_date)
        .ok_or_else(absent)?;
    // DELETE validates the nominal restoration, rather than treating undo as a
    // second override or requiring the old override to survive validation.
    guard(state, bounds, occurrence.start, occurrence.end)?;
    objective
        .validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut payload = original.clone();
    payload["recurrence"] = serde_json::to_value(&objective.recurrence).map_err(internal)?;
    let now = state.planning_now();
    let changed = payload != original;
    let admitted_version = if changed {
        let envelope = state.envelope_for(
            [(
                objective.id.clone(),
                VersionRef::Version(u64::try_from(row.version).map_err(internal)?),
            )]
            .into_iter()
            .collect(),
            AuthoritySource::User,
            now,
        )?;
        queries::admit_object(
            pool,
            &envelope,
            NewObjectRecord {
                id: row.id.clone(),
                object_type: row.object_type,
                version: row
                    .version
                    .checked_add(1)
                    .ok_or_else(|| internal("Objective version exhausted"))?,
                status: row.status,
                compartment_label: row.compartment_label,
                payload,
                created_at: row.created_at,
                updated_at: now.to_string(),
            },
        )
        .await?
        .version
    } else {
        row.version
    };
    super::routine_service::refresh_override_window(state, occurrence, now).await?;
    if changed {
        let envelope = state.envelope_for(
            [(
                objective.id.clone(),
                VersionRef::Version(u64::try_from(admitted_version).map_err(internal)?),
            )]
            .into_iter()
            .collect(),
            AuthoritySource::User,
            now,
        )?;
        let mut log = json!({"schema_version":"ubu.orchestrator.routine_override.v1","action":if window.is_some(){"set_occurrence_override"}else{"clear_occurrence_override"},"local_date":local_date,"window":window.map(|(start,end)|json!({"start":start,"end":end}))});
        if let Some(source) = source {
            log["source"] = json!({"source_kind":"google_calendar","source_id":source.external_id});
            log["task_id"] = source.task_id.into();
        }
        // The decision changes an Objective. It is not Task execution evidence.
        queries::append_log_entry(
            pool,
            &envelope,
            NewLogRecord {
                id: UbuId::new(ObjectType::LogEntry).to_string(),
                event_type: "decision_recorded".into(),
                object_refs: json!([objective_id]),
                payload: log,
                provenance: json!({"created_at":now,"authority_source":"user"}),
                created_at: now.to_string(),
            },
        )
        .await?;
    }
    Ok(OverrideOutcome {
        objective_id: objective_id.into(),
        local_date: local_date.into(),
        overridden: window.is_some(),
        diagnostics: derived.diagnostics,
    })
}
