use serde_json::json;
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::queries;

use crate::api::user_action::{
    LogEntryResponse, RecordedTaskActionKind, RecordedTaskActionRequest,
    RecordedTaskActionResponse, TaskActionKind, TaskLifecycleStatus, UserActionRequest,
    TASK_ACTION_SCHEMA_VERSION,
};
use crate::errors::{AppError, Result};
use crate::services::recalculation_service;
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
    let transition_applied = if matches!(request.action, RecordedTaskActionKind::Complete) {
        apply_completed_transition(pool, &mut task).await?;
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

    let mut diagnostics = Vec::new();
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
