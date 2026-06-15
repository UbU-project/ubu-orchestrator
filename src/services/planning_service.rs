use ubu_planning_core::{PlanningRequest, TaskSpec, PLANNING_SCHEMA_VERSION};

use crate::adapters::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::api::calendar::CalendarResponse;
use crate::api::next_action::NextActionResponse;
use crate::api::planning::{GeneratePlanningRequest, PlanningResponseBody, ScheduledTaskBody};
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn generate(
    state: AppState,
    request: GeneratePlanningRequest,
) -> Result<PlanningResponseBody> {
    let planning_request = match request.request {
        Some(request) => request.into(),
        None => build_request_from_imports(&state).await?,
    };

    let adapter = CpuPlannerAdapter;
    let response = adapter.plan(planning_request.clone());
    let body = PlanningResponseBody::from(response.clone());

    let mut memory = state.inner().memory.lock().await;
    memory.planning_request = Some(planning_request);
    memory.admitted_plan = response.plan.clone();
    memory.planning_response = Some(response);
    memory.next_action = body
        .plan
        .as_ref()
        .and_then(|plan| plan.tasks.first())
        .map(|task| NextActionResponse {
            task_id: task.task_id.clone(),
            title: task.task_id.clone(),
            status: TaskLifecycleStatus::Active,
            readiness: true,
            start: task.start,
            end: task.end,
        });

    Ok(body)
}

pub async fn current_calendar(state: AppState) -> Result<CalendarResponse> {
    let memory = state.inner().memory.lock().await;
    let Some(plan) = &memory.admitted_plan else {
        return Ok(CalendarResponse {
            plan_id: None,
            tasks: Vec::new(),
        });
    };

    Ok(CalendarResponse {
        plan_id: Some(plan.plan_id.clone()),
        tasks: plan
            .tasks
            .iter()
            .map(|task| ScheduledTaskBody {
                task_id: task.task_id.clone(),
                start: task.start,
                end: task.end,
                depends_on: task.depends_on.clone(),
                static_anchor: task.static_anchor,
            })
            .collect(),
    })
}

async fn build_request_from_imports(state: &AppState) -> Result<PlanningRequest> {
    let memory = state.inner().memory.lock().await;
    if memory.imported_candidates.is_empty() {
        return Err(AppError::BadRequest(
            "import GitHub candidates before generating a plan".to_owned(),
        ));
    }

    Ok(PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_owned()),
        request_id: "fixture-loop".to_owned(),
        tasks: memory
            .imported_candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| TaskSpec {
                id: candidate.task_id.clone(),
                duration: 30,
                depends_on: Vec::new(),
                window: None,
                static_anchor: None,
                affect_required: index == 0,
                affect_current: index == 0,
            })
            .collect(),
    })
}
