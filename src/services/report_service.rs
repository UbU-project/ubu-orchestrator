use sqlx::Row;

use crate::api::reports::{
    CategoryTime, HumanCompleteReportResponse, RiskLevel, RiskReportResponse, TaskStatusCount,
    TimeByCategoryResponse, UnmeasuredTask, TIME_BY_CATEGORY_SCHEMA_VERSION,
};
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
use crate::services::calendar_interaction;
use crate::state::AppState;

pub async fn risk(state: AppState) -> Result<RiskReportResponse> {
    Ok(
        crate::services::planning_service::latest_admitted_plan(&state)
            .await?
            .and_then(|plan| plan.risk_report)
            .unwrap_or_else(|| RiskReportResponse {
                generated_at: ubu_core::UbuTimestamp::now_utc().to_string(),
                level: RiskLevel::Low,
                findings: Vec::new(),
            }),
    )
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

// ---------------------------------------------------------------- time by category
//
// Quick UbU's `report_by_category`, with one change mainline needs: a Task
// contributes at most once, from its latest completion, and only while it is
// still completed, so a complete, reopen, complete cannot count twice.

/// The report's default span when `from` is absent: Quick UbU's `--days 7`.
pub const DEFAULT_REPORT_SPAN_SECONDS: i64 = 7 * 86_400;
pub const UNCATEGORIZED: &str = "Uncategorized";

fn parse_bound(name: &str, value: &str) -> Result<ubu_core::UbuTimestamp> {
    ubu_core::UbuTimestamp::parse(value).map_err(|_| {
        AppError::bad_request_diagnostic(
            "time_by_category_invalid_bound",
            format!("`{name}` must be an RFC 3339 instant, not `{value}`"),
        )
    })
}

/// The number of whole seconds of the estimate that stands in for a completion
/// with no observed window: the fixed figure, or the mode of a skewed estimate.
fn estimate_seconds(estimate: &serde_json::Value) -> Option<u64> {
    match estimate["type"].as_str()? {
        "fixed" => estimate["seconds"].as_u64(),
        "shifted_lognormal_p95" => estimate["mode_seconds"].as_u64(),
        _ => None,
    }
}

pub async fn time_by_category(
    state: &AppState,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<TimeByCategoryResponse> {
    use std::collections::BTreeMap;
    use ubu_core::UbuTimestamp;

    let now = state.planning_now();
    let to = match to {
        Some(value) => parse_bound("to", value)?,
        None => now,
    };
    let from = match from {
        Some(value) => parse_bound("from", value)?,
        None => {
            let seconds = u64::try_from(to.inner().unix_timestamp() - DEFAULT_REPORT_SPAN_SECONDS)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            UbuTimestamp::parse(crate::planning_time::timestamp_at(seconds)?)
                .map_err(|e| AppError::Internal(e.to_string()))?
        }
    };
    if from > to {
        return Err(AppError::bad_request_diagnostic(
            "time_by_category_invalid_range",
            format!("`from` ({from}) is after `to` ({to}); nothing can be reported for a range that ends before it starts"),
        ));
    }
    let (from_s, to_s) = (from.inner().unix_timestamp(), to.inner().unix_timestamp());

    let pool = state.inner().store.pool();
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, status, payload_json FROM objects WHERE object_type='Task' AND status IN ('active','completed') ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    struct Row {
        static_seconds: u64,
        completed_seconds: u64,
        task_count: usize,
    }
    let mut categories: BTreeMap<String, Row> = BTreeMap::new();
    let mut unmeasured = Vec::new();
    for (task_id, status, raw) in rows {
        let payload: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.to_string()))?;
        let category = payload["category_tag"]
            .as_str()
            .filter(|tag| !tag.trim().is_empty())
            .unwrap_or(UNCATEGORIZED)
            .to_owned();
        let title = payload["title"].as_str().unwrap_or("").to_owned();
        let window = payload["static_window"]["start"]
            .as_str()
            .zip(payload["static_window"]["end"].as_str())
            .and_then(|(start, end)| UbuTimestamp::parse(start).ok().zip(UbuTimestamp::parse(end).ok()));

        let contribution = if let Some((start, end)) = window {
            // Static: occupied time is occupied, ticked or not.
            let overlap = end.inner().unix_timestamp().min(to_s) - start.inner().unix_timestamp().max(from_s);
            (overlap > 0).then(|| (u64::try_from(overlap).unwrap_or(0), 0))
        } else if status == "completed" {
            // Dynamic: the latest completion, and only while the Task is still completed.
            let Some(completion) = calendar_interaction::latest_completion(pool, &task_id).await? else {
                continue;
            };
            let Ok(recorded) = UbuTimestamp::parse(&completion.recorded_at) else {
                continue;
            };
            let at = recorded.inner().unix_timestamp();
            if at < from_s || at > to_s {
                continue;
            }
            match completion.observed_seconds.or_else(|| estimate_seconds(&payload["duration_estimate"])) {
                Some(seconds) => Some((0, seconds)),
                None => {
                    unmeasured.push(UnmeasuredTask {
                        task_id: task_id.clone(),
                        title,
                        reason: "completed with no observed window and no duration estimate; the time it took is not recorded".into(),
                    });
                    None
                }
            }
        } else {
            None
        };
        if let Some((static_seconds, completed_seconds)) = contribution {
            let row = categories.entry(category).or_insert(Row { static_seconds: 0, completed_seconds: 0, task_count: 0 });
            row.static_seconds += static_seconds;
            row.completed_seconds += completed_seconds;
            row.task_count += 1;
        }
    }

    let mut categories: Vec<CategoryTime> = categories
        .into_iter()
        .map(|(category, row)| CategoryTime {
            category,
            seconds: row.static_seconds + row.completed_seconds,
            static_seconds: row.static_seconds,
            completed_seconds: row.completed_seconds,
            task_count: row.task_count,
        })
        .collect();
    // Biggest first; equal seconds in category order, so the order is stable.
    categories.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.category.cmp(&b.category)));
    let total_seconds = categories.iter().map(|row| row.seconds).sum();
    Ok(TimeByCategoryResponse {
        schema_version: TIME_BY_CATEGORY_SCHEMA_VERSION.into(),
        generated_at: now.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        categories,
        unmeasured,
        total_seconds,
    })
}

