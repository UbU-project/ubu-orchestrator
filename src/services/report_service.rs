use crate::api::reports::{HumanCompleteReportResponse, RiskReportResponse, TaskStatusCount};
use crate::api::user_action::{TaskActionKind, TaskLifecycleStatus};
use crate::errors::Result;
use crate::state::AppState;

pub async fn risk(state: AppState) -> Result<RiskReportResponse> {
    let memory = state.inner().memory.lock().await;
    let mut risks = Vec::new();
    if memory.imported_candidates.is_empty() {
        risks.push("no_github_candidates_imported".to_owned());
    }
    if memory.admitted_plan.is_none() {
        risks.push("no_plan_admitted".to_owned());
    }
    Ok(RiskReportResponse {
        risks,
        task_statuses: task_status_counts(&memory.log_entries),
    })
}

pub async fn human_complete(state: AppState) -> Result<HumanCompleteReportResponse> {
    let memory = state.inner().memory.lock().await;
    let completed_tasks = memory
        .log_entries
        .iter()
        .filter(|entry| matches!(entry.action, TaskActionKind::Done))
        .count();
    Ok(HumanCompleteReportResponse {
        completed_tasks,
        task_statuses: task_status_counts(&memory.log_entries),
        notes: memory
            .log_entries
            .iter()
            .filter_map(|entry| entry.note.clone())
            .collect(),
    })
}

fn task_status_counts(
    entries: &[crate::api::user_action::LogEntryResponse],
) -> Vec<TaskStatusCount> {
    [
        TaskLifecycleStatus::Active,
        TaskLifecycleStatus::Completed,
        TaskLifecycleStatus::Failed,
        TaskLifecycleStatus::Moot,
    ]
    .into_iter()
    .map(|status| TaskStatusCount {
        status,
        count: entries
            .iter()
            .filter(|entry| entry.status == status)
            .count(),
    })
    .collect()
}
