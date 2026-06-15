use serde_json::json;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::queries;

use crate::api::user_action::{
    LogEntryResponse, TaskActionKind, TaskLifecycleStatus, UserActionRequest,
};
use crate::errors::{AppError, Result};
use crate::services::recalculation_service;
use crate::state::AppState;

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
