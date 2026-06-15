use std::collections::HashSet;

use sqlx::Row;
use ubu_planning_core::Plan;

use crate::api::next_action::NextActionResponse;
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn get_next_action(state: AppState) -> Result<NextActionResponse> {
    let pool = state.inner().store.pool();

    let row = sqlx::query("SELECT payload_json FROM plans ORDER BY created_at DESC LIMIT 1")
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some(row) = row else {
        return Err(AppError::NotFound(
            "no plan available; run /planning/generate first".to_owned(),
        ));
    };

    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let plan: Plan = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize plan: {e}")))?;

    let terminal_ids = terminal_task_ids(pool).await?;

    let first = plan
        .tasks
        .into_iter()
        .find(|task| !terminal_ids.contains(&task.task_id))
        .ok_or_else(|| AppError::NotFound("no active tasks available".to_owned()))?;

    let title = task_title(pool, &first.task_id).await?;

    Ok(NextActionResponse {
        task_id: first.task_id,
        title,
        status: TaskLifecycleStatus::Active,
        readiness: true,
        start: first.start,
        end: first.end,
    })
}

async fn terminal_task_ids(pool: &sqlx::SqlitePool) -> Result<HashSet<String>> {
    let rows = sqlx::query(
        "SELECT object_refs_json FROM logs WHERE event_type IN ('task_done', 'task_rejected', 'task_failed')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut ids = HashSet::new();
    for row in rows {
        let refs_json: String = row
            .try_get("object_refs_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if let Ok(refs) = serde_json::from_str::<Vec<String>>(&refs_json) {
            ids.extend(refs);
        }
    }
    Ok(ids)
}

async fn task_title(pool: &sqlx::SqlitePool, task_id: &str) -> Result<String> {
    let row = sqlx::query("SELECT payload_json FROM objects WHERE id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if let Some(row) = row {
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&payload_json) {
            if let Some(title) = payload["title"].as_str() {
                return Ok(title.to_owned());
            }
        }
    }
    Ok(task_id.to_owned())
}
