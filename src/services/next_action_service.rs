use std::collections::HashSet;

use serde_json::Value;
use ubu_core::id_registry::ObjectType;
use ubu_store::models::object_record::ObjectRecord;

use crate::api::next_action::{
    NextActionDiagnostic, NextActionDiagnosticCode, NextActionExplanation, NextActionObjectiveRef,
    NextActionRecommendation, NextActionRequest, NextActionResponse, NextActionSelection,
    NextActionSourceRef, ReadinessState, NEXT_ACTION_SCHEMA_VERSION,
};
use crate::api::planning::{AffectLegitimizationModeBody, LegitimizationReportBody, PlanBody};
use crate::api::user_action::TaskLifecycleStatus;
use crate::errors::{AppError, Result};
use crate::services::planning_service;
use crate::state::AppState;

pub async fn get_next_action(
    state: AppState,
    request: NextActionRequest,
) -> Result<NextActionResponse> {
    validate_schema_version(request.schema_version.as_deref())?;

    let pool = state.inner().store.pool();
    let current_calendar = planning_service::latest_admitted_plan(&state)
        .await?
        .filter(|plan| plan.legitimization.is_some());

    if let Some(plan) = current_calendar.as_ref() {
        if matches!(
            plan.legitimization.as_ref(),
            Some(report)
                if report.mode == AffectLegitimizationModeBody::Enforce
                    && report.result == "failed"
        ) {
            return Ok(failed_legitimization_response(plan));
        }
    }

    let tasks = load_tasks(pool).await?;
    if tasks.is_empty() {
        return Ok(diagnostic_response(NextActionDiagnostic {
            code: NextActionDiagnosticCode::NoAdmittedTasks,
            message: "no admitted Tasks are available for readiness selection".to_owned(),
            blocked_task_count: 0,
            sampled_task_ids: Vec::new(),
        }));
    }

    let active_tasks: Vec<_> = tasks
        .iter()
        .filter(|task| task.record.status == "active")
        .collect();
    if active_tasks.is_empty() {
        return Ok(diagnostic_response(NextActionDiagnostic {
            code: NextActionDiagnosticCode::NoActiveTasks,
            message: "admitted Tasks exist, but none are active".to_owned(),
            blocked_task_count: 0,
            sampled_task_ids: Vec::new(),
        }));
    }

    let completed_ids: HashSet<String> = tasks
        .iter()
        .filter(|task| task.record.status == "completed")
        .map(|task| task.record.id.clone())
        .collect();

    let mut candidates = active_tasks
        .iter()
        .map(|task| {
            let readiness = evaluate_readiness(&task.payload, &completed_ids);
            (*task, readiness)
        })
        .collect::<Vec<_>>();

    if let Some(plan) = current_calendar.as_ref() {
        let Some(first_placement) = plan.steps.iter().min_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| left.end.cmp(&right.end))
                .then_with(|| left.task_id.cmp(&right.task_id))
        }) else {
            return Ok(diagnostic_response(NextActionDiagnostic {
                code: NextActionDiagnosticCode::NoReadyTask,
                message: "the current legitimized Calendar has no Task placements".to_owned(),
                blocked_task_count: 0,
                sampled_task_ids: Vec::new(),
            }));
        };

        let Some(task) = active_tasks
            .iter()
            .find(|task| task.record.id == first_placement.task_id)
        else {
            return Ok(diagnostic_response(NextActionDiagnostic {
                code: NextActionDiagnosticCode::NoReadyTask,
                message: "the first placement in the current legitimized Calendar is not an active admitted Task"
                    .to_owned(),
                blocked_task_count: 1,
                sampled_task_ids: vec![first_placement.task_id.clone()],
            }));
        };

        return recommendation_response(
            pool,
            task,
            NextActionSelection {
                rule: "legitimized_calendar_first_placement".to_owned(),
                priority: explicit_priority(&task.payload),
                tiebreak: "start ascending, then end ascending, then task_id ascending".to_owned(),
            },
            plan.legitimization.as_ref().and_then(calendar_warning),
        )
        .await;
    }

    candidates.sort_by(|(left, _), (right, _)| {
        explicit_priority(&left.payload)
            .unwrap_or(i64::MAX)
            .cmp(&explicit_priority(&right.payload).unwrap_or(i64::MAX))
            .then_with(|| left.record.created_at.cmp(&right.record.created_at))
            .then_with(|| left.record.id.cmp(&right.record.id))
    });

    if let Some((task, _)) = candidates
        .iter()
        .find(|(_, readiness)| readiness.state == ReadinessState::Ready)
    {
        return recommendation_response(
            pool,
            task,
            NextActionSelection {
                rule: "readiness_ordered_skeleton".to_owned(),
                priority: explicit_priority(&task.payload),
                tiebreak:
                    "explicit priority ascending, then created_at ascending, then task_id ascending"
                        .to_owned(),
            },
            None,
        )
        .await;
    }

    let blocked_task_count = candidates.len();
    let sampled_task_ids = candidates
        .iter()
        .take(3)
        .map(|(task, _)| task.record.id.clone())
        .collect();
    let all_dependency_blocked = candidates
        .iter()
        .all(|(_, readiness)| readiness.reasons == vec![BlockedReason::UnmetDependencies]);
    let all_precondition_blocked = candidates
        .iter()
        .all(|(_, readiness)| readiness.reasons == vec![BlockedReason::UnmetPreconditions]);
    let (code, message) = if all_dependency_blocked {
        (
            NextActionDiagnosticCode::AllCandidatesBlockedOnUnmetDependencies,
            "all active Task candidates are blocked on unmet dependencies",
        )
    } else if all_precondition_blocked {
        (
            NextActionDiagnosticCode::AllCandidatesBlockedOnPreconditions,
            "all active Task candidates are blocked on deterministic preconditions",
        )
    } else {
        (
            NextActionDiagnosticCode::NoReadyTask,
            "no active Task candidate is ready under the bounded readiness rules",
        )
    };

    Ok(diagnostic_response(NextActionDiagnostic {
        code,
        message: message.to_owned(),
        blocked_task_count,
        sampled_task_ids,
    }))
}

fn failed_legitimization_response(plan: &PlanBody) -> NextActionResponse {
    let sampled_task_ids = plan
        .steps
        .iter()
        .map(|step| step.task_id.clone())
        .take(3)
        .collect();
    diagnostic_response(NextActionDiagnostic {
        code: NextActionDiagnosticCode::NoReadyTask,
        message: "the current Calendar failed affect legitimization under enforce mode; no Task can be recommended"
            .to_owned(),
        blocked_task_count: plan.steps.len(),
        sampled_task_ids,
    })
}

async fn recommendation_response(
    pool: &sqlx::SqlitePool,
    task: &TaskRow,
    selection: NextActionSelection,
    warning: Option<String>,
) -> Result<NextActionResponse> {
    let source_refs = source_refs(&task.payload);
    let parent_objective = parent_objective(pool, &task.payload).await?;
    let from_calendar = selection.rule == "legitimized_calendar_first_placement";
    let explanation = explanation(
        parent_objective.clone(),
        source_refs.clone(),
        from_calendar,
        warning,
    );

    Ok(NextActionResponse {
        schema_version: NEXT_ACTION_SCHEMA_VERSION.to_owned(),
        recommendation: Some(NextActionRecommendation {
            task_id: task.record.id.clone(),
            title: task_title(&task.payload, &task.record.id),
            status: TaskLifecycleStatus::Active,
            readiness: ReadinessState::Ready,
            parent_objective,
            source_refs,
            selection,
            explanation,
        }),
        diagnostics: Vec::new(),
    })
}

fn calendar_warning(report: &LegitimizationReportBody) -> Option<String> {
    if report.mode != AffectLegitimizationModeBody::WarnOnly {
        return None;
    }

    let mut details = Vec::new();
    if report.result != "passed" {
        details.push(format!("legitimization result `{}`", report.result));
    }
    if !report.violated_dimensions.is_empty() {
        details.push(format!(
            "violated affect dimensions: {}",
            report.violated_dimensions.join(", ")
        ));
    }
    if let Some(warning) = report.stale_affect_warning.as_ref() {
        details.push(warning.clone());
    }

    (!details.is_empty()).then(|| format!("Calendar warning (warn_only): {}.", details.join("; ")))
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(NEXT_ACTION_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

fn diagnostic_response(diagnostic: NextActionDiagnostic) -> NextActionResponse {
    NextActionResponse {
        schema_version: NEXT_ACTION_SCHEMA_VERSION.to_owned(),
        recommendation: None,
        diagnostics: vec![diagnostic],
    }
}

#[derive(Debug)]
struct TaskRow {
    record: ObjectRecord,
    payload: Value,
}

async fn load_tasks(pool: &sqlx::SqlitePool) -> Result<Vec<TaskRow>> {
    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type = ? ORDER BY created_at ASC, id ASC",
    )
    .bind(ObjectType::Task.as_str())
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    rows.into_iter()
        .map(|record| {
            let payload = serde_json::from_str(&record.payload_json)
                .map_err(|e| AppError::Internal(format!("failed to deserialize task: {e}")))?;
            Ok(TaskRow { record, payload })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockedReason {
    UnmetDependencies,
    UnmetPreconditions,
}

#[derive(Debug)]
struct Readiness {
    state: ReadinessState,
    reasons: Vec<BlockedReason>,
}

fn evaluate_readiness(payload: &Value, completed_ids: &HashSet<String>) -> Readiness {
    let dependencies = dependencies(payload);
    let dependencies_met = dependencies
        .iter()
        .all(|dependency| completed_ids.contains(dependency));
    let preconditions_met = deterministic_preconditions_met(payload);

    let mut reasons = Vec::new();
    if !dependencies_met {
        reasons.push(BlockedReason::UnmetDependencies);
    }
    if !preconditions_met {
        reasons.push(BlockedReason::UnmetPreconditions);
    }

    Readiness {
        state: if reasons.is_empty() {
            ReadinessState::Ready
        } else {
            ReadinessState::Blocked
        },
        reasons,
    }
}

fn dependencies(payload: &Value) -> Vec<String> {
    ["blocked_by", "depends_on", "dependencies"]
        .iter()
        .filter_map(|field| payload.get(field).and_then(Value::as_array))
        .flat_map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn deterministic_preconditions_met(payload: &Value) -> bool {
    if matches!(
        payload.get("precondition_satisfied"),
        Some(Value::Bool(false))
    ) {
        return false;
    }

    let Some(preconditions) = payload.get("preconditions") else {
        return true;
    };

    match preconditions {
        Value::Array(items) => items.iter().all(|item| match item {
            Value::Bool(value) => *value,
            Value::Object(map) => matches!(map.get("satisfied"), Some(Value::Bool(true))),
            _ => false,
        }),
        Value::Bool(value) => *value,
        _ => false,
    }
}

fn explicit_priority(payload: &Value) -> Option<i64> {
    payload.get("priority").and_then(Value::as_i64)
}

fn task_title(payload: &Value, fallback_id: &str) -> String {
    payload
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(fallback_id)
        .to_owned()
}

async fn parent_objective(
    pool: &sqlx::SqlitePool,
    payload: &Value,
) -> Result<Option<NextActionObjectiveRef>> {
    if let Some(objective_id) = payload.get("objective_id").and_then(Value::as_str) {
        return objective_by_id(pool, objective_id).await;
    }

    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type = ? AND status = ? ORDER BY created_at ASC, id ASC LIMIT 2",
    )
    .bind(ObjectType::Objective.as_str())
    .bind("active")
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    if rows.len() == 1 {
        return objective_from_record(&rows[0]).map(Some);
    }

    Ok(None)
}

async fn objective_by_id(
    pool: &sqlx::SqlitePool,
    objective_id: &str,
) -> Result<Option<NextActionObjectiveRef>> {
    let row = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE id = ? AND object_type = ? LIMIT 1",
    )
    .bind(objective_id)
    .bind(ObjectType::Objective.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    row.as_ref().map(objective_from_record).transpose()
}

fn objective_from_record(record: &ObjectRecord) -> Result<NextActionObjectiveRef> {
    let payload: Value = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize objective: {e}")))?;
    Ok(NextActionObjectiveRef {
        objective_id: record.id.clone(),
        title: task_title(&payload, &record.id),
    })
}

fn source_refs(payload: &Value) -> Vec<NextActionSourceRef> {
    let Some(provenance) = payload.get("provenance") else {
        return Vec::new();
    };

    let mut refs = Vec::new();
    if let Some(source_refs) = provenance.get("source_refs").and_then(Value::as_array) {
        refs.extend(source_refs.iter().filter_map(source_ref));
    }
    if let Some(source) = provenance.get("source").and_then(source_ref) {
        refs.push(source);
    }
    refs
}

fn source_ref(value: &Value) -> Option<NextActionSourceRef> {
    Some(NextActionSourceRef {
        source_kind: value.get("source_kind")?.as_str()?.to_owned(),
        source_id: value.get("source_id")?.as_str()?.to_owned(),
        url: value.get("url").and_then(Value::as_str).map(str::to_owned),
    })
}

fn explanation(
    parent_objective: Option<NextActionObjectiveRef>,
    source_refs: Vec<NextActionSourceRef>,
    from_calendar: bool,
    warning: Option<String>,
) -> NextActionExplanation {
    let objective_text = parent_objective
        .as_ref()
        .map(|objective| format!("parent Objective '{}'", objective.title))
        .unwrap_or_else(|| "no parent Objective recorded".to_owned());
    let provenance_text = if source_refs.is_empty() {
        "no provenance source_refs recorded".to_owned()
    } else {
        format!("{} provenance source reference(s)", source_refs.len())
    };

    let warning_text = warning
        .map(|warning| format!(" {warning}"))
        .unwrap_or_default();
    let selection_text = if from_calendar {
        "selected the first Task placement in the current legitimized Calendar"
    } else {
        "selected a ready Task"
    };

    NextActionExplanation {
        template_id: "readiness_based_recommendation.v1".to_owned(),
        label: "readiness-based recommendation".to_owned(),
        message: format!(
            "Readiness-based recommendation: {selection_text} linked to {objective_text} with {provenance_text}.{warning_text}"
        ),
        readiness_state: ReadinessState::Ready,
        parent_objective,
        source_refs,
    }
}
