use std::collections::HashSet;

use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::queries;

use crate::adapters::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::api::planning::{DiagnosticBody, RepairScopeBody};
use crate::api::recalculation::{
    RecalculationRequest, RecalculationResponse, RecalculationTriggerTypeBody,
    RECALCULATION_SCHEMA_VERSION,
};
use crate::errors::{AppError, Result};
use crate::services::planning_service;
use crate::state::AppState;

pub async fn recalculate(state: AppState) -> Result<()> {
    let request = RecalculationRequest {
        schema_version: Some(RECALCULATION_SCHEMA_VERSION.to_owned()),
        triggered_at: UbuTimestamp::now_utc().to_string(),
        trigger_type: RecalculationTriggerTypeBody::WorkerRequest,
        note: Some("legacy recalculation request".to_owned()),
        objects: Vec::new(),
    };
    recalculate_from_request(state, request).await.map(|_| ())
}

pub async fn recalculate_from_request(
    state: AppState,
    request: RecalculationRequest,
) -> Result<RecalculationResponse> {
    validate_schema_version(request.schema_version.as_deref())?;
    UbuTimestamp::parse(&request.triggered_at)
        .map_err(|e| AppError::bad_request_diagnostic("invalid_triggered_at", e.to_string()))?;
    append_recalculation_log(&state, &request).await?;

    let prior_plan = planning_service::latest_admitted_plan(&state)
        .await?
        .ok_or_else(|| {
            AppError::bad_request_diagnostic(
                "missing_prior_plan",
                "recalculation requires an admitted prior Plan",
            )
        })?;
    let repair_scope = repair_scope(request.trigger_type);
    let frozen_task_ids = frozen_task_ids(&state, &prior_plan.id).await?;
    let frozen_steps = planning_service::frozen_steps_for_plan(&prior_plan, &frozen_task_ids);
    assert_override_safety(&prior_plan, &frozen_steps, &frozen_task_ids)?;

    let repair_request_body = planning_service::build_repair_request_from_store(
        &state,
        &prior_plan,
        repair_scope,
        request
            .objects
            .iter()
            .map(|object| object.id.clone())
            .collect(),
        &frozen_task_ids.iter().cloned().collect::<Vec<_>>(),
    )
    .await?;
    let adapter = CpuPlannerAdapter;
    let repair_response = adapter.repair(planning_service::repair_kernel_request(
        &repair_request_body,
    ));
    let diagnostics = repair_response
        .diagnostics
        .into_iter()
        .map(|diagnostic| DiagnosticBody {
            code: format!("{:?}", diagnostic.code),
            message: diagnostic.message,
        })
        .collect::<Vec<_>>();

    let plan = match repair_response.repaired_plan {
        Some(plan) => {
            let stored = planning_service::persist_repair_plan(
                &state,
                &repair_request_body,
                &plan,
                &prior_plan,
                frozen_steps,
            )
            .await?;
            planning_service::supersede_plan(&state, &prior_plan.id).await?;
            Some(stored)
        }
        None => None,
    };

    Ok(RecalculationResponse {
        schema_version: RECALCULATION_SCHEMA_VERSION.to_owned(),
        trigger_type: request.trigger_type,
        repair_scope,
        prior_plan_id: prior_plan.id,
        plan,
        diagnostics,
    })
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        None | Some(RECALCULATION_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
    }
}

fn repair_scope(trigger_type: RecalculationTriggerTypeBody) -> RepairScopeBody {
    match trigger_type {
        RecalculationTriggerTypeBody::TaskFailed => RepairScopeBody::FailedTask,
        RecalculationTriggerTypeBody::TaskMoot => RepairScopeBody::MootTask,
        RecalculationTriggerTypeBody::UserOverride => RepairScopeBody::OverridePlacement,
        RecalculationTriggerTypeBody::TaskCompleted
        | RecalculationTriggerTypeBody::ObservedSnapshot
        | RecalculationTriggerTypeBody::ExternalEvent
        | RecalculationTriggerTypeBody::GithubUpdate
        | RecalculationTriggerTypeBody::LowCompactCalendarCoverage
        | RecalculationTriggerTypeBody::WorkerRequest => RepairScopeBody::RemainingWindow,
    }
}

async fn append_recalculation_log(state: &AppState, request: &RecalculationRequest) -> Result<()> {
    let now = UbuTimestamp::now_utc().to_string();
    let mut payload = json!({
        "triggered_at": request.triggered_at,
        "trigger_type": trigger_type_wire(request.trigger_type),
    });
    if let Some(note) = &request.note {
        payload["note"] = json!(note);
    }
    if !request.objects.is_empty() {
        payload["objects"] = json!(request.objects);
    }
    let object_ref_ids = request
        .objects
        .iter()
        .map(|object| object.id.clone())
        .collect::<Vec<_>>();

    queries::append_log_entry(
        state.inner().store.pool(),
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "recalculation_requested".to_owned(),
            object_refs: json!(object_ref_ids),
            payload,
            provenance: json!({
                "created_at": now,
                "authority_source": "system"
            }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;
    Ok(())
}

async fn frozen_task_ids(state: &AppState, prior_plan_id: &str) -> Result<HashSet<String>> {
    let pool = state.inner().store.pool();
    let mut frozen = HashSet::new();

    let rows = sqlx::query("SELECT id, status FROM objects WHERE object_type = ?")
        .bind(ObjectType::Task.as_str())
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let status: String = row
            .try_get("status")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if matches!(
            status.as_str(),
            "completed" | "failed" | "moot" | "in_progress"
        ) {
            frozen.insert(id);
        }
    }

    let rows = sqlx::query(
        "SELECT event_type, object_refs_json, provenance_json
        FROM logs
        ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    for row in rows {
        let event_type: String = row
            .try_get("event_type")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let object_refs_json: String = row
            .try_get("object_refs_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let provenance_json: String = row
            .try_get("provenance_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let object_refs = task_refs(&object_refs_json)?;
        let provenance: Value = serde_json::from_str(&provenance_json)
            .map_err(|e| AppError::Internal(format!("failed to deserialize provenance: {e}")))?;
        let is_user_override = provenance
            .get("authority_source")
            .and_then(Value::as_str)
            .is_some_and(|authority| authority == "user_override");

        if is_user_override
            || matches!(
                event_type.as_str(),
                "task_started" | "task_done" | "task_failed" | "task_rejected"
            )
        {
            frozen.extend(object_refs);
        }
    }

    let prior_plan = planning_service::latest_admitted_plan(state).await?;
    if prior_plan
        .as_ref()
        .is_some_and(|plan| plan.id.as_str() == prior_plan_id)
    {
        let planned_ids = prior_plan
            .unwrap()
            .steps
            .into_iter()
            .map(|step| step.task_id)
            .collect::<HashSet<_>>();
        frozen.retain(|task_id| planned_ids.contains(task_id));
    }

    Ok(frozen)
}

fn assert_override_safety(
    prior_plan: &crate::api::planning::PlanBody,
    frozen_steps: &[crate::api::planning::ScheduledTaskBody],
    frozen_task_ids: &HashSet<String>,
) -> Result<()> {
    let frozen_step_ids = frozen_steps
        .iter()
        .map(|step| step.task_id.clone())
        .collect::<HashSet<_>>();
    for task_id in frozen_task_ids {
        if prior_plan.steps.iter().any(|step| &step.task_id == task_id)
            && !frozen_step_ids.contains(task_id)
        {
            return Err(AppError::Internal(format!(
                "frozen task `{task_id}` lost its prior placement during recalculation"
            )));
        }
    }
    Ok(())
}

fn task_refs(object_refs_json: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(object_refs_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize object refs: {e}")))?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .filter(|id| id.starts_with("task_"))
        .collect())
}

fn trigger_type_wire(trigger_type: RecalculationTriggerTypeBody) -> &'static str {
    match trigger_type {
        RecalculationTriggerTypeBody::TaskCompleted => "task_completed",
        RecalculationTriggerTypeBody::TaskFailed => "task_failed",
        RecalculationTriggerTypeBody::TaskMoot => "task_moot",
        RecalculationTriggerTypeBody::UserOverride => "user_override",
        RecalculationTriggerTypeBody::ObservedSnapshot => "observed_snapshot",
        RecalculationTriggerTypeBody::ExternalEvent => "external_event",
        RecalculationTriggerTypeBody::GithubUpdate => "github_update",
        RecalculationTriggerTypeBody::LowCompactCalendarCoverage => "low_compact_calendar_coverage",
        RecalculationTriggerTypeBody::WorkerRequest => "worker_request",
    }
}
