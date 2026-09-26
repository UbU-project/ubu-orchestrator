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
    /// Prevent pre-partition category colours exported by UbU from completing work.
    pub exported_uncoloured: bool,
    pub latest_completion: Option<CompletionRecord>,
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
            exported_uncoloured: old.color_id.is_none(),
            latest_completion: latest_completion(pool, &task.id).await?,
        });
    }
    Ok(owned)
}

pub struct InteractionResult {
    pub changed_external_ids: BTreeSet<String>,
    pub diagnostics: Vec<DiagnosticBody>,
}

pub async fn apply(
    state: &AppState,
    observed: &[DesiredEvent],
    applied: &mut [DesiredEvent],
) -> Result<InteractionResult> {
    let owned = observed_owned(state.inner().store.pool(), observed, applied).await?;
    let (completions, reopens, diagnostics) = detect(&owned, state.planning_now());
    let mut result = InteractionResult {
        changed_external_ids: BTreeSet::new(),
        diagnostics,
    };
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
        if !item.is_static
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
