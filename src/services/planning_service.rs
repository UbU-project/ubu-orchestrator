use sqlx::Row;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_planning_core::{Plan, PlanningRequest, TaskSpec, PLANNING_SCHEMA_VERSION};
use ubu_store::models::plan_record::NewPlanRecord;
use ubu_store::queries;

use crate::adapters::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::api::calendar::CalendarResponse;
use crate::api::planning::{GeneratePlanningRequest, PlanningResponseBody, ScheduledTaskBody};
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn generate(
    state: AppState,
    request: GeneratePlanningRequest,
) -> Result<PlanningResponseBody> {
    let planning_request = match request.request {
        Some(body) => body.into(),
        None => build_request_from_store(&state).await?,
    };

    let adapter = CpuPlannerAdapter;
    let response = adapter.plan(planning_request);
    let body = PlanningResponseBody::from(response.clone());

    if let Some(plan) = &response.plan {
        let pool = state.inner().store.pool();
        let plan_id = UbuId::new(ObjectType::Plan).to_string();
        let now = UbuTimestamp::now_utc().to_string();
        queries::store_plan(
            pool,
            NewPlanRecord {
                id: plan_id,
                request_id: response.request_id.clone(),
                status: "admitted".to_owned(),
                payload: serde_json::to_value(plan)
                    .map_err(|e| AppError::Internal(e.to_string()))?,
                created_at: now,
            },
        )
        .await
        .map_err(AppError::from)?;
    }

    Ok(body)
}

pub async fn current_calendar(state: AppState) -> Result<CalendarResponse> {
    let pool = state.inner().store.pool();
    let row =
        sqlx::query("SELECT payload_json FROM plans ORDER BY created_at DESC LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some(row) = row else {
        return Ok(CalendarResponse {
            plan_id: None,
            tasks: Vec::new(),
        });
    };

    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let plan: Plan = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize plan: {e}")))?;

    Ok(CalendarResponse {
        plan_id: Some(plan.plan_id),
        tasks: plan
            .tasks
            .into_iter()
            .map(|task| ScheduledTaskBody {
                task_id: task.task_id,
                start: task.start,
                end: task.end,
                depends_on: task.depends_on,
                static_anchor: task.static_anchor,
            })
            .collect(),
    })
}

async fn build_request_from_store(state: &AppState) -> Result<PlanningRequest> {
    let pool = state.inner().store.pool();
    let tasks = queries::query_active_tasks(pool)
        .await
        .map_err(AppError::from)?;

    if tasks.is_empty() {
        return Err(AppError::BadRequest(
            "import GitHub candidates before generating a plan".to_owned(),
        ));
    }

    Ok(PlanningRequest {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_owned()),
        request_id: "store-backed".to_owned(),
        tasks: tasks
            .iter()
            .enumerate()
            .map(|(index, record)| TaskSpec {
                id: record.id.clone(),
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
