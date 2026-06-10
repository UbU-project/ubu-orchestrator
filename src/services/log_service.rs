use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId};

use crate::api::user_action::{LogEntryResponse, TaskActionKind, UserActionRequest};
use crate::errors::Result;
use crate::services::recalculation_service;
use crate::state::AppState;

pub async fn append_action(
    state: AppState,
    task_id: String,
    action: TaskActionKind,
    request: UserActionRequest,
) -> Result<LogEntryResponse> {
    let entry = LogEntryResponse {
        log_id: UbuId::new(ObjectType::LogEntry).to_string(),
        task_id,
        action,
        authority_source: format!("{:?}", AuthoritySource::User).to_ascii_lowercase(),
        note: request.note,
    };

    {
        let mut memory = state.inner().memory.lock().await;
        memory.log_entries.push(entry.clone());
    }

    recalculation_service::recalculate(state).await?;
    Ok(entry)
}
