use sqlx::Row;

use crate::api::reports::{HumanCompleteReportResponse, RiskReportResponse, TaskStatusCount};
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn risk(state: AppState) -> Result<RiskReportResponse> {
    let pool = state.inner().store.pool();

    let active_tasks = ubu_store::queries::query_active_tasks(pool)
        .await
        .map_err(AppError::from)?;

    let plan_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans")
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut risks = Vec::new();
    if active_tasks.is_empty() {
        risks.push("no_github_candidates_imported".to_owned());
    }
    if plan_count == 0 {
        risks.push("no_plan_admitted".to_owned());
    }

    Ok(RiskReportResponse {
        risks,
        task_statuses: task_status_counts(pool).await?,
    })
}

pub async fn human_complete(state: AppState) -> Result<HumanCompleteReportResponse> {
    let pool = state.inner().store.pool();

    let completed_tasks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type = 'task_done'")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let notes = notes_from_logs(pool).await?;

    Ok(HumanCompleteReportResponse {
        completed_tasks: completed_tasks as usize,
        task_statuses: task_status_counts(pool).await?,
        notes,
    })
}

async fn task_status_counts(pool: &sqlx::SqlitePool) -> Result<Vec<TaskStatusCount>> {
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM logs WHERE event_type IN ('task_started', 'task_snoozed', 'task_decomposed')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let completed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type = 'task_done'")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let failed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type = 'task_failed'")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let moot: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type = 'task_rejected'")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(vec![
        TaskStatusCount {
            status: TaskLifecycleStatus::Active,
            count: active as usize,
        },
        TaskStatusCount {
            status: TaskLifecycleStatus::Completed,
            count: completed as usize,
        },
        TaskStatusCount {
            status: TaskLifecycleStatus::Failed,
            count: failed as usize,
        },
        TaskStatusCount {
            status: TaskLifecycleStatus::Moot,
            count: moot as usize,
        },
    ])
}

async fn notes_from_logs(pool: &sqlx::SqlitePool) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT payload_json FROM logs WHERE event_type LIKE 'task_%'")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut notes = Vec::new();
    for row in rows {
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&payload_json) {
            if let Some(note) = payload["note"].as_str() {
                notes.push(note.to_owned());
            }
        }
    }
    Ok(notes)
}
