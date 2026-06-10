use crate::api::reports::{HumanCompleteReportResponse, RiskReportResponse};
use crate::api::user_action::TaskActionKind;
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
    Ok(RiskReportResponse { risks })
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
        notes: memory
            .log_entries
            .iter()
            .filter_map(|entry| entry.note.clone())
            .collect(),
    })
}
