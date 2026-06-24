use serde_json::json;
use ubu_core::core::{apply_universe_mutations, validate_mutations_for_mode, InstanceMode, TaskEffect};
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::queries;

use crate::api::user_action::{
    ActionDiagnostic, LogEntryResponse, RecordedTaskActionKind, RecordedTaskActionRequest,
    RecordedTaskActionResponse, TaskActionKind, TaskLifecycleStatus, UserActionRequest,
    TASK_ACTION_SCHEMA_VERSION,
};
use crate::errors::{AppError, Result};
use crate::instance_mode::MVP_INSTANCE_MODE;
use crate::services::{planning_service, recalculation_service};
use crate::state::AppState;

pub async fn record_task_action(
    state: AppState,
    task_id: String,
    request: RecordedTaskActionRequest,
) -> Result<RecordedTaskActionResponse> {
    validate_schema_version(request.schema_version.as_deref())?;

    let pool = state.inner().store.pool();
    let mut task = load_task(pool, &task_id).await?;
    let authority_source = authority_for_recorded_action(request.action);
    let mut diagnostics = Vec::new();
    let transition_applied = if matches!(request.action, RecordedTaskActionKind::Complete) {
        apply_completed_transition(pool, &mut task).await?;
        // A completed Task applies its effects to UniverseState; the effect
        // applies because the Task completed, so success_probability is ignored.
        diagnostics.extend(
            apply_completed_effects(pool, &task, authority_source, MVP_INSTANCE_MODE).await?,
        );
        true
    } else {
        false
    };

    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let authority_source_wire = authority_source_wire(authority_source)?;
    let task_status = task_status_from_wire(&task.status)?;

    let mut payload = json!({
        "schema_version": TASK_ACTION_SCHEMA_VERSION,
        "action": recorded_action_wire(request.action),
        "decision": recorded_decision_wire(request.action),
        "task_status": task.status,
        "transition_applied": transition_applied,
    });
    if let Some(note) = &request.note {
        payload["note"] = json!(note);
    }

    // TODO(O6-task-transition-log-event): Replace this decision_recorded fallback
    // with a dedicated canonical task-transition Log event after a recorded
    // decision ticket extends the closed LogEventType vocabulary.
    queries::append_log_entry(
        pool,
        NewLogRecord {
            id: log_id.clone(),
            event_type: "decision_recorded".to_owned(),
            object_refs: json!([task_id.clone()]),
            payload,
            provenance: json!({
                "created_at": now,
                "authority_source": authority_source_wire
            }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    if matches!(request.action, RecordedTaskActionKind::Snooze) {
        // TODO(O6-snooze-readiness): Snooze records a defer decision only;
        // snooze-aware readiness is intentionally deferred out of this slice.
        diagnostics.push(crate::api::user_action::ActionDiagnostic {
            code: "snooze_readiness_deferred".to_owned(),
            message: "snooze was recorded without changing readiness evaluation".to_owned(),
        });
    }

    Ok(RecordedTaskActionResponse {
        schema_version: TASK_ACTION_SCHEMA_VERSION.to_owned(),
        log_id,
        task_id,
        action: request.action,
        task_status,
        authority_source: authority_source_wire,
        transition_applied,
        note: request.note,
        diagnostics,
    })
}

pub async fn append_action(
    state: AppState,
    task_id: String,
    action: TaskActionKind,
    request: UserActionRequest,
) -> Result<LogEntryResponse> {
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let event_type = action_event_type(action);
    let status = TaskLifecycleStatus::for_action(action);

    let mut payload = json!({
        "action": format!("{action:?}").to_ascii_lowercase(),
        "task_status": format!("{status:?}").to_ascii_lowercase(),
    });
    if let Some(note) = &request.note {
        payload["note"] = json!(note);
    }

    queries::append_log_entry(
        state.inner().store.pool(),
        NewLogRecord {
            id: log_id.clone(),
            event_type,
            object_refs: json!([task_id]),
            payload,
            provenance: json!({
                "created_at": now,
                "authority_source": "user"
            }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    recalculation_service::recalculate(state).await?;

    Ok(LogEntryResponse {
        log_id,
        task_id,
        action,
        status,
        authority_source: "user".to_owned(),
        note: request.note,
    })
}

fn action_event_type(action: TaskActionKind) -> String {
    match action {
        TaskActionKind::Start => "task_started",
        TaskActionKind::Done => "task_done",
        TaskActionKind::Snooze => "task_snoozed",
        TaskActionKind::Reject => "task_rejected",
        TaskActionKind::Decompose => "task_decomposed",
    }
    .to_owned()
}

struct TaskForTransition {
    id: String,
    status: String,
    payload: serde_json::Value,
}

async fn load_task(pool: &sqlx::SqlitePool, task_id: &str) -> Result<TaskForTransition> {
    let record = queries::get_current_state(pool, task_id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::NotFound(format!("task `{task_id}` not found")))?;

    if record.object_type != ObjectType::Task.as_str() {
        return Err(AppError::bad_request_diagnostic(
            "not_a_task",
            format!("object `{task_id}` is not a Task"),
        ));
    }

    let payload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize task: {e}")))?;

    Ok(TaskForTransition {
        id: record.id,
        status: record.status,
        payload,
    })
}

async fn apply_completed_transition(
    pool: &sqlx::SqlitePool,
    task: &mut TaskForTransition,
) -> Result<()> {
    if task.status != "active" {
        return Err(AppError::bad_request_diagnostic(
            "invalid_task_state",
            "complete can only transition an active Task",
        ));
    }

    let now = UbuTimestamp::now_utc().to_string();
    task.status = "completed".to_owned();
    task.payload["status"] = json!("completed");
    let payload_json = serde_json::to_string(&task.payload)
        .map_err(|e| AppError::Internal(format!("failed to serialize task: {e}")))?;

    sqlx::query(
        "UPDATE objects
        SET status = ?, payload_json = ?, updated_at = ?, version = version + 1
        WHERE id = ?",
    )
    .bind(&task.status)
    .bind(payload_json)
    .bind(now)
    .bind(&task.id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(())
}

/// Apply a completed Task's effects to the current `UniverseState` (§10.2).
///
/// The effect applies because the Task completed, so `success_probability` is
/// planning metadata only and never gates application here. Effects mutations
/// are validated against the instance mode (Wiring-B) and applied with the pure
/// `ubu-core` applicator (C9); the result is persisted as a new current version
/// through the store's `persist_universe_state` (ST7) under the completing
/// action's authority. No SQL is written against UniverseState. Mode or
/// application failures surface as diagnostics without partially persisting.
///
/// Only completed transitions reach this path; a Task that transitions to
/// `failed` applies nothing.
async fn apply_completed_effects(
    pool: &sqlx::SqlitePool,
    task: &TaskForTransition,
    authority_source: AuthoritySource,
    mode: InstanceMode,
) -> Result<Vec<ActionDiagnostic>> {
    let Some(effects_value) = task.payload.get("effects") else {
        return Ok(Vec::new());
    };
    let effect: TaskEffect = serde_json::from_value(effects_value.clone())
        .map_err(|e| AppError::Internal(format!("failed to deserialize task effects: {e}")))?;

    // `success_probability` is intentionally ignored: completion, not the
    // predicted probability, is what makes the effect apply.
    if effect.mutations.is_empty() {
        return Ok(Vec::new());
    }

    // Mode validation: an intrinsic-affect mutation target under a mode that
    // does not model intrinsic affect makes the effect invalid (surfaced
    // distinctly); `user_mode` always permits.
    if let Err(error) = validate_mutations_for_mode(mode, &effect.mutations) {
        return Ok(vec![ActionDiagnostic {
            code: "task_effect_mode_invalid".to_owned(),
            message: error.to_string(),
        }]);
    }

    let Some(current_state) = planning_service::read_current_universe_state(pool).await? else {
        return Ok(vec![ActionDiagnostic {
            code: "task_effect_universe_state_absent".to_owned(),
            message: "no current UniverseState exists; completed Task effects were not applied"
                .to_owned(),
        }]);
    };

    let next_state = match apply_universe_mutations(&current_state, &effect.mutations) {
        Ok(next_state) => next_state,
        Err(error) => {
            return Ok(vec![ActionDiagnostic {
                code: "task_effect_application_failed".to_owned(),
                message: error.to_string(),
            }]);
        }
    };

    queries::persist_universe_state(pool, &next_state, authority_source)
        .await
        .map_err(AppError::from)?;

    Ok(Vec::new())
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(TASK_ACTION_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

fn authority_for_recorded_action(action: RecordedTaskActionKind) -> AuthoritySource {
    match action {
        RecordedTaskActionKind::Complete | RecordedTaskActionKind::Snooze => AuthoritySource::User,
        RecordedTaskActionKind::Override => AuthoritySource::UserOverride,
    }
}

fn authority_source_wire(authority_source: AuthoritySource) -> Result<String> {
    let serialized =
        serde_json::to_string(&authority_source).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(serialized.trim_matches('"').to_owned())
}

fn recorded_action_wire(action: RecordedTaskActionKind) -> &'static str {
    match action {
        RecordedTaskActionKind::Complete => "complete",
        RecordedTaskActionKind::Override => "override",
        RecordedTaskActionKind::Snooze => "snooze",
    }
}

fn recorded_decision_wire(action: RecordedTaskActionKind) -> &'static str {
    match action {
        RecordedTaskActionKind::Complete => "task_completed",
        RecordedTaskActionKind::Override => "recommendation_rejected",
        RecordedTaskActionKind::Snooze => "defer",
    }
}

fn task_status_from_wire(status: &str) -> Result<TaskLifecycleStatus> {
    match status {
        "active" => Ok(TaskLifecycleStatus::Active),
        "completed" => Ok(TaskLifecycleStatus::Completed),
        "failed" => Ok(TaskLifecycleStatus::Failed),
        "moot" => Ok(TaskLifecycleStatus::Moot),
        other => Err(AppError::Internal(format!(
            "stored Task has unsupported lifecycle status `{other}`"
        ))),
    }
}

#[cfg(test)]
mod effect_mode_tests {
    use super::*;
    use ubu_store::UbuStore;

    fn completed_task_with_effects(effects: serde_json::Value) -> TaskForTransition {
        TaskForTransition {
            id: UbuId::new(ObjectType::Task).to_string(),
            status: "completed".to_owned(),
            payload: json!({ "effects": effects }),
        }
    }

    fn intrinsic_affect_effect() -> serde_json::Value {
        json!({
            "mutations": [
                {
                    "operation": "increment_numeric",
                    "target": "numeric_values.affect.energy",
                    "payload": 1.0
                }
            ]
        })
    }

    #[tokio::test]
    async fn organization_mode_rejects_intrinsic_affect_effect() {
        let store = UbuStore::in_memory().await.expect("store");
        let task = completed_task_with_effects(intrinsic_affect_effect());
        let diagnostics = apply_completed_effects(
            store.pool(),
            &task,
            AuthoritySource::User,
            InstanceMode::OrganizationMode,
        )
        .await
        .expect("effects evaluated");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "task_effect_mode_invalid");
    }

    #[tokio::test]
    async fn user_mode_permits_intrinsic_affect_effect() {
        // user_mode models intrinsic affect, so the mode check passes; with no
        // current UniverseState the effect simply has nowhere to persist.
        let store = UbuStore::in_memory().await.expect("store");
        let task = completed_task_with_effects(intrinsic_affect_effect());
        let diagnostics = apply_completed_effects(
            store.pool(),
            &task,
            AuthoritySource::User,
            InstanceMode::UserMode,
        )
        .await
        .expect("effects evaluated");
        assert_eq!(diagnostics[0].code, "task_effect_universe_state_absent");
    }
}
