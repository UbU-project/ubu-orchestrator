//! Pure Calendar gestures, followed by ordinary action admission.
use super::{calendar_projection::DesiredEvent, calendar_range::CalendarTimeRange, log_service};
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::Value;
use std::collections::BTreeSet;
use ubu_core::UbuTimestamp;
use ubu_store::queries;

#[derive(Debug, Clone)]
pub struct CompletionSignal {
    pub task_id: String,
    pub external_id: String,
    pub observed_start: String,
    pub observed_end: String,
    /// Preserve the pre-observation Dynamic colour meaning when this pass pins it.
    pub allow_occurrence_pin: bool,
}
#[derive(Debug, Clone)]
pub struct ReopenSignal {
    pub task_id: String,
    pub external_id: String,
    pub completion_log_id: String,
}
#[derive(Debug, Clone)]
pub struct CompletionRecord {
    pub log_id: String,
    pub source_kind: Option<String>,
    pub source_id: Option<String>,
    pub has_observed_window: bool,
}
impl CompletionRecord {
    pub fn is_from_calendar_event(&self, external_id: &str) -> bool {
        self.source_kind.as_deref() == Some("google_calendar")
            && self.source_id.as_deref() == Some(external_id)
            && self.has_observed_window
    }
}
#[derive(Debug, Clone)]
pub struct ObservedOwned {
    pub event: DesiredEvent,
    pub task_id: String,
    pub status: String,
    pub is_static: bool,
    pub is_captured: bool,
    pub static_window: Option<(String, String)>,
    pub declared_duration_seconds: u64,
    pub routine_objective_id: Option<String>,
    /// The last observed/exported window distinguishes an occurrence gesture
    /// from an unchanged event already planned using its derived model.
    pub applied_window: (String, String),
    /// Prevent pre-partition category colours exported by UbU from completing work.
    pub exported_uncoloured: bool,
    pub latest_completion: Option<CompletionRecord>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MoveSignal {
    pub task_id: String,
    pub external_id: String,
    pub new_start: String,
    pub new_end: String,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResizeSignal {
    pub task_id: String,
    pub external_id: String,
    pub new_duration_seconds: u64,
}

/// Placement decides the gesture; reading canonical state and admission stay outside.
pub fn detect_window_gestures(
    owned: &[ObservedOwned],
) -> (Vec<MoveSignal>, Vec<ResizeSignal>, Vec<DiagnosticBody>) {
    let mut moves = Vec::new();
    let mut resizes = Vec::new();
    let mut diagnostics = Vec::new();
    for item in owned {
        let window = (item.event.start_at.as_str(), item.event.end_at.as_str());
        let changed = !same_window(window, (&item.applied_window.0, &item.applied_window.1));
        if item.status != "active" {
            if changed || item.static_window.as_ref().is_some_and(|old| !same_window(window, (&old.0, &old.1))) {
                diagnostics.push(DiagnosticBody {
                    code: "calendar_gesture_on_inactive_task".into(),
                    message: format!("Task `{}` is {}; Calendar window gesture ignored", item.task_id, item.status),
                });
            }
            continue;
        }
        if item.routine_objective_id.is_some() {
            if changed {
                moves.push(MoveSignal {
                    task_id: item.task_id.clone(), external_id: item.event.external_id.clone(),
                    new_start: item.event.start_at.clone(), new_end: item.event.end_at.clone(),
                });
            }
            continue;
        }
        if item.is_static {
            if item.static_window.as_ref().is_some_and(|old| !same_window(window, (&old.0, &old.1))) {
                moves.push(MoveSignal {
                    task_id: item.task_id.clone(), external_id: item.event.external_id.clone(),
                    new_start: item.event.start_at.clone(), new_end: item.event.end_at.clone(),
                });
            }
        } else {
            match CalendarTimeRange::parse(&item.event.start_at, &item.event.end_at) {
                Ok(range) => {
                    let seconds = (range.end.inner().unix_timestamp() - range.start.inner().unix_timestamp()) as u64;
                    if seconds > 0 && seconds != item.declared_duration_seconds {
                        resizes.push(ResizeSignal {
                            task_id: item.task_id.clone(), external_id: item.event.external_id.clone(),
                            new_duration_seconds: seconds,
                        });
                    }
                }
                Err(_) => diagnostics.push(DiagnosticBody {
                    code: "calendar_resize_invalid_window".into(),
                    message: format!("Task `{}` has no positive observed duration; resize ignored", item.task_id),
                }),
            }
        }
    }
    (moves, resizes, diagnostics)
}

fn same_window(left: (&str, &str), right: (&str, &str)) -> bool {
    [(left.0, right.0), (left.1, right.1)].into_iter().all(|(a, b)| {
        UbuTimestamp::parse(a).ok().zip(UbuTimestamp::parse(b).ok())
            .is_some_and(|(a, b)| a == b)
    })
}

pub fn detect(
    owned: &[ObservedOwned],
    now: UbuTimestamp,
) -> (
    Vec<CompletionSignal>,
    Vec<ReopenSignal>,
    Vec<DiagnosticBody>,
) {
    let mut completions = Vec::new();
    let mut reopens = Vec::new();
    let mut diagnostics = Vec::new();
    for item in owned {
        if item.is_static || item.is_captured {
            continue;
        }
        let complete =
            item.status != "completed" && item.event.color_id.is_some() && item.exported_uncoloured;
        let reopen = item.status == "completed"
            && item.event.color_id.is_none()
            && item.latest_completion.as_ref().is_some_and(|completion| {
                completion.is_from_calendar_event(&item.event.external_id)
            });
        if !complete && !reopen {
            continue;
        }
        let Ok(window) = CalendarTimeRange::parse(&item.event.start_at, &item.event.end_at) else {
            diagnostics.push(DiagnosticBody {
                code: "capture_interaction_invalid_window".into(),
                message: format!(
                    "Task `{}` event `{}` has no valid observed window; gesture ignored",
                    item.task_id, item.event.external_id
                ),
            });
            continue;
        };
        if complete {
            completions.push(CompletionSignal {
                task_id: item.task_id.clone(),
                external_id: item.event.external_id.clone(),
                observed_start: item.event.start_at.clone(),
                observed_end: item.event.end_at.clone(),
                allow_occurrence_pin: item.routine_objective_id.is_some(),
            });
        } else if window.end.inner().unix_timestamp()
            > now.inner().unix_timestamp().saturating_sub(86_400)
        {
            reopens.push(ReopenSignal {
                task_id: item.task_id.clone(),
                external_id: item.event.external_id.clone(),
                completion_log_id: item.latest_completion.as_ref().unwrap().log_id.clone(),
            });
        }
    }
    (completions, reopens, diagnostics)
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// Append order resolves ties from a fixed clock and includes legacy app `done` facts.
pub async fn latest_completion(
    pool: &sqlx::SqlitePool,
    task_id: &str,
) -> Result<Option<CompletionRecord>> {
    let row: Option<(String,String)> = sqlx::query_as("SELECT id,payload_json FROM logs WHERE (event_type='task_done' OR (event_type='decision_recorded' AND json_extract(payload_json,'$.decision')='task_completed')) AND EXISTS (SELECT 1 FROM json_each(logs.object_refs_json) WHERE value=?) ORDER BY rowid DESC LIMIT 1")
        .bind(task_id).fetch_optional(pool).await.map_err(internal)?;
    row.map(|(log_id, raw)| {
        let payload: Value = serde_json::from_str(&raw).map_err(internal)?;
        Ok(CompletionRecord {
            log_id,
            source_kind: payload["source"]["source_kind"].as_str().map(str::to_owned),
            source_id: payload["source"]["source_id"].as_str().map(str::to_owned),
            has_observed_window: payload["observed_window"]["start"]
                .as_str()
                .zip(payload["observed_window"]["end"].as_str())
                .is_some_and(|(start, end)| CalendarTimeRange::parse(start, end).is_ok()),
        })
    })
    .transpose()
}

pub async fn observed_owned(
    pool: &sqlx::SqlitePool,
    observed: &[DesiredEvent],
    applied: &[DesiredEvent],
) -> Result<Vec<ObservedOwned>> {
    let mut owned = Vec::new();
    let mut seen = BTreeSet::new();
    for event in observed {
        let Some(old) = applied
            .iter()
            .find(|old| old.external_id == event.external_id)
        else {
            continue;
        };
        if !seen.insert(old.task_id.clone()) {
            continue;
        }
        let Some(task) = queries::get_current_state(pool, &old.task_id).await? else {
            continue;
        };
        if task.object_type != "Task" {
            continue;
        }
        let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
        owned.push(ObservedOwned {
            event: event.clone(),
            task_id: task.id.clone(),
            status: task.status,
            is_static: payload
                .get("static_window")
                .is_some_and(|window| !window.is_null()),
            is_captured: payload["provenance"]["source"]["source_kind"] == "google_calendar",
            static_window: payload["static_window"]["start"].as_str()
                .zip(payload["static_window"]["end"].as_str())
                .map(|(start, end)| (start.to_owned(), end.to_owned())),
            declared_duration_seconds: payload["duration_estimate"]["seconds"].as_u64()
                .or_else(|| payload["duration_estimate"]["mode_seconds"].as_u64())
                .unwrap_or_else(|| super::planning_service::duration_seconds(&payload)),
            routine_objective_id: payload["occurrence"]["routine_objective_id"].as_str().map(str::to_owned),
            applied_window: (old.start_at.clone(), old.end_at.clone()),
            exported_uncoloured: old.color_id.is_none(),
            latest_completion: latest_completion(pool, &task.id).await?,
        });
    }
    Ok(owned)
}

pub struct InteractionResult {
    pub changed_external_ids: BTreeSet<String>,
    pub moved: usize,
    pub resized: usize,
    pub diagnostics: Vec<DiagnosticBody>,
}

pub async fn apply(
    state: &AppState,
    observed: &[DesiredEvent],
    applied: &mut [DesiredEvent],
) -> Result<InteractionResult> {
    let owned = observed_owned(state.inner().store.pool(), observed, applied).await?;
    let (completions, reopens, diagnostics) = detect(&owned, state.planning_now());
    let (moves, resizes, window_diagnostics) = detect_window_gestures(&owned);
    let mut result = InteractionResult {
        moved: 0,
        resized: 0,
        changed_external_ids: BTreeSet::new(),
        diagnostics,
    };
    result.diagnostics.extend(window_diagnostics);
    for signal in moves {
        match apply_move(state, &signal).await {
            Ok(diagnostics) => {
                result.diagnostics.extend(diagnostics);
                result.moved += 1;
                accept_observation(&signal.external_id, observed, applied);
                result.changed_external_ids.insert(signal.external_id);
            }
            Err(AppError::Diagnostic { code, message, .. }) => result.diagnostics.push(DiagnosticBody { code, message }),
            Err(AppError::Diagnostics { items, .. }) => result.diagnostics.extend(items.into_iter().map(|(code, message)| DiagnosticBody { code, message })),
            Err(error) => return Err(error),
        }
    }
    for signal in resizes {
        match apply_resize(state, &signal).await {
            Ok(()) => {
                result.resized += 1;
                accept_observation(&signal.external_id, observed, applied);
                result.changed_external_ids.insert(signal.external_id);
            }
            Err(AppError::Diagnostic { code, message, .. }) => result.diagnostics.push(DiagnosticBody { code, message }),
            Err(AppError::Diagnostics { items, .. }) => result.diagnostics.extend(items.into_iter().map(|(code, message)| DiagnosticBody { code, message })),
            Err(error) => return Err(error),
        }
    }
    for signal in completions {
        match log_service::record_calendar_completion(state.clone(), &signal).await {
            Ok(response) => {
                result
                    .diagnostics
                    .extend(response.diagnostics.into_iter().map(|d| DiagnosticBody {
                        code: d.code,
                        message: d.message,
                    }));
                accept_observation(&signal.external_id, observed, applied);
                result.changed_external_ids.insert(signal.external_id);
            }
            Err(AppError::Diagnostic { code, message, .. }) => {
                result.diagnostics.push(DiagnosticBody { code, message })
            }
            Err(error) => return Err(error),
        }
    }
    for signal in reopens {
        if log_service::reopen_calendar_completion(state, &signal).await? {
            accept_observation(&signal.external_id, observed, applied);
            result.changed_external_ids.insert(signal.external_id);
        }
    }
    Ok(result)
}

fn accept_observation(external_id: &str, observed: &[DesiredEvent], applied: &mut [DesiredEvent]) {
    if let (Some(old), Some(current)) = (
        applied.iter_mut().find(|e| e.external_id == external_id),
        observed.iter().find(|e| e.external_id == external_id),
    ) {
        *old = current.clone();
    }
}

/// Keep phone-completed events available for the bounded undo gesture, even
/// after regeneration removes the Task or an old plan still contains it.
pub async fn preserve_completed(
    pool: &sqlx::SqlitePool,
    desired: &mut Vec<DesiredEvent>,
    applied: &[DesiredEvent],
) -> Result<()> {
    let owned = observed_owned(pool, applied, applied).await?;
    for item in owned {
        if (!item.is_static || item.routine_objective_id.is_some())
            && !item.is_captured
            && item.status == "completed"
            && item.latest_completion.as_ref().is_some_and(|completion| {
                completion.is_from_calendar_event(&item.event.external_id)
            })
        {
            desired.retain(|event| event.task_id != item.task_id);
            desired.push(item.event);
        }
    }
    desired.sort_by(|a, b| (&a.start_at, &a.external_id).cmp(&(&b.start_at, &b.external_id)));
    Ok(())
}


async fn apply_move(state: &AppState, signal: &MoveSignal) -> Result<Vec<DiagnosticBody>> {
    if let Some(task) = queries::get_current_state(state.inner().store.pool(), &signal.task_id).await? {
        let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
        if let Some((objective, date)) = payload["occurrence"]["routine_objective_id"].as_str().zip(payload["occurrence"]["local_date"].as_str()) {
            let window = CalendarTimeRange::parse(&signal.new_start, &signal.new_end)
                .map_err(|_| AppError::bad_request_diagnostic("routine_override_invalid_window", "Override window must have a strictly positive span"))?;
            let result = super::routine_override::change(state, objective, date, Some((window.start, window.end)), Some(super::routine_override::CalendarSource {
                task_id: &signal.task_id, external_id: &signal.external_id,
            })).await?;
            return Ok(result.diagnostics);
        }
    }
    let _guard = state.inner().task_action_lock.lock().await;
    let task = queries::get_current_state(state.inner().store.pool(), &signal.task_id).await?
        .ok_or_else(|| AppError::NotFound(format!("Task `{}` disappeared", signal.task_id)))?;
    let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
    let reject = |code: &str, reason: &str| AppError::bad_request_diagnostic(code,
        format!("Task `{}` event `{}`: {reason}; move rejected", signal.task_id, signal.external_id));
    if task.status != "active" {
        return Err(reject("calendar_gesture_on_inactive_task", "Task is no longer active"));
    }
    if payload["static_window"].is_null() {
        return Err(reject("calendar_move_not_static", "Task is no longer Static"));
    }
    let range = CalendarTimeRange::parse(&signal.new_start, &signal.new_end)
        .map_err(|_| reject("calendar_move_invalid_window", "window must have a strictly positive span"))?;
    let created = payload["provenance"]["created_at"].as_str()
        .and_then(|value| UbuTimestamp::parse(value).ok())
        .ok_or_else(|| reject("calendar_move_invalid_creation", "Task creation time is unavailable"))?;
    if range.start < created {
        return Err(reject("calendar_move_before_creation", "window starts before Task creation"));
    }
    let horizon = CalendarTimeRange::planning(state).await?;
    let latest_end = horizon.end.inner().unix_timestamp().saturating_add(state.inner().planning_horizon_seconds as i64);
    if range.end.inner().unix_timestamp() > latest_end {
        return Err(reject("calendar_move_beyond_horizon", "window ends beyond the planning horizon plus one configured horizon length"));
    }
    super::task_capture::edit(state, &task.id, task.version,
        serde_json::json!({"static_window":{"start":signal.new_start,"end":signal.new_end}})).await?;
    log_service::append_calendar_move(state, signal).await?;
    Ok(Vec::new())
}

/// Static placement belongs to the current Task even when the stored plan predates
/// its edit. Never project an obsolete meeting window back onto the operator.
pub async fn current_static_windows(pool: &sqlx::SqlitePool, desired: &mut [DesiredEvent]) -> Result<()> {
    for event in desired.iter_mut() {
        let Some(task) = queries::get_current_state(pool, &event.task_id).await? else { continue; };
        if task.status != "active" { continue; }
        let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
        if let Some((start, end)) = payload["static_window"]["start"].as_str().zip(payload["static_window"]["end"].as_str()) {
            let window = CalendarTimeRange::parse(start, end).map_err(internal)?;
            event.start_at = crate::planning_time::timestamp_at(u64::try_from(window.start.inner().unix_timestamp()).map_err(internal)?)?;
            event.end_at = crate::planning_time::timestamp_at(u64::try_from(window.end.inner().unix_timestamp()).map_err(internal)?)?;
        }
    }
    desired.sort_by(|a, b| (&a.start_at, &a.external_id).cmp(&(&b.start_at, &b.external_id)));
    Ok(())
}


async fn apply_resize(state: &AppState, signal: &ResizeSignal) -> Result<()> {
    let _guard = state.inner().task_action_lock.lock().await;
    let task = queries::get_current_state(state.inner().store.pool(), &signal.task_id).await?
        .ok_or_else(|| AppError::NotFound(format!("Task `{}` disappeared", signal.task_id)))?;
    if task.status != "active" {
        return Err(AppError::bad_request_diagnostic("calendar_gesture_on_inactive_task",
            format!("Task `{}` is no longer active; resize ignored", signal.task_id)));
    }
    let payload: Value = serde_json::from_str(&task.payload_json).map_err(internal)?;
    if !payload["static_window"].is_null() {
        return Err(AppError::bad_request_diagnostic("calendar_resize_not_dynamic",
            format!("Task `{}` is no longer Dynamic; resize ignored", signal.task_id)));
    }
    super::task_capture::edit(state, &task.id, task.version,
        serde_json::json!({"duration_estimate":{"type":"fixed","seconds":signal.new_duration_seconds}})).await?;
    Ok(())
}
