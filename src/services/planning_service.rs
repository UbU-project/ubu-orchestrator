use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::core::{
    evaluate_universe_precondition, validate_precondition_for_mode, InstanceMode,
    UniversePrecondition, UniversePreconditionError, UniverseState,
};
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_planning_core::{
    CorrelationGroup, DurationModel, Plan as KernelPlan, PlanCandidate, PlanStatus,
    PlanningRequest, RepairRequest, ScheduledTask, TaskSpec, TimeWindow, PLANNING_SCHEMA_VERSION,
};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::models::plan_record::NewPlanRecord;
use ubu_core::core::{Preference, AllowedTimeRange, StaticWindow, TaskCorrelationGroup, TaskDurationEstimate};
use ubu_store::queries;

use crate::adapters::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::api::calendar::CalendarResponse;
use crate::api::planning::{
    candidate_role_body, feasibility_summary_body, legitimization_report_body,
    probability_quality_body, score_summary_body, semi_legitimization_summary_body,
    AffectDirectionBody, AffectLegitimizationModeBody, AffectObservationBody,
    AffectObservationValueBody, AffectProfileBody, AffectToleranceBody, BlockedTaskBody,
    ComputeBudgetBody, CorrelationGroupBody, DiagnosticBody, DurationEstimateBody,
    GeneratePlanningRequest, InvalidTaskBody, LegitimizationReportBody, PlanBody,
    PlanCandidateBody, PlanningHorizonBody, PlanningModeBody, PlanningRequestBody, PlanningResponseBody,
    ProbabilityQualityBody, RepairContextBody, RepairScopeBody, ScheduledTaskBody,
    ScoringPolicyBody, StaticAnchorBody, TaskGraphBody, TaskGraphEdgeBody, TaskSpecBody,
    TimeWindowBody, TaskPriorityBody, UnplacedTaskBody,
};
use crate::errors::{AppError, Result};
use crate::reports::planning_analysis::{self, PlanningAnalysisInput};
use crate::state::AppState;

/// Fixed planning duration used when a stored Task has no duration estimate.
const DEFAULT_TASK_DURATION_SECONDS: u64 = 1800;
const DEFAULT_AFFECT_SCALE: f64 = 1.5;
const DEFAULT_AFFECT_THRESHOLD: f64 = 0.5;

pub async fn generate(
    state: AppState,
    request: GeneratePlanningRequest,
) -> Result<PlanningResponseBody> {
    validate_optional_schema_version(request.schema_version.as_deref())?;
    let StorePlanningRequest {
        request: planning_request,
        blocked_tasks,
        invalid_tasks,
        mut diagnostics,
        task_priorities,
        non_capacity_tasks,
        covered_static_tasks,
        carrier_windows,
    } = match request.request {
        Some(body) => {
            validate_optional_schema_version(body.schema_version.as_deref())?;
            StorePlanningRequest {
                request: body,
                blocked_tasks: Vec::new(),
                invalid_tasks: Vec::new(),
                diagnostics: Vec::new(),
                task_priorities: Vec::new(),
                non_capacity_tasks: Vec::new(),
                covered_static_tasks: Vec::new(),
                carrier_windows: HashMap::new(),
            }
        }
        None => {
            build_request_from_store_with_context(
                &state,
                PlanningModeBody::FreshGeneration,
                None,
                &[],
                request.horizon.as_ref(),
            )
            .await?
        }
    };

    let direct = DirectPlacements { non_capacity: &non_capacity_tasks, covered: &covered_static_tasks, carriers: &carrier_windows };
    validate_task_models(&planning_request)?;
    let kernel_request = PlanningRequest::from(planning_request.clone());
    let adapter = CpuPlannerAdapter {
        strategy: state.inner().planner_strategy,
    };
    add_empty_capacity_diagnostic(&planning_request, &mut diagnostics);
    let mut kernel_unplaced = Vec::new();
    let mut candidates = if has_static_conflicts(&diagnostics) || planning_request.tasks.is_empty() {
        Vec::new()
    } else {
        let response = adapter.plan(kernel_request.clone());
        kernel_unplaced = response.unplaced_tasks;
        diagnostics.extend(diagnostics_from_kernel(response.diagnostics));
        response.plan_candidates
    };
    diagnostics.extend(precondition_diagnostics(&blocked_tasks, &invalid_tasks));
    let titles = task_titles(state.inner().store.pool(), &state.inner().category_palette).await?;
    let unplaced_tasks = kernel_unplaced.iter().map(|entry| UnplacedTaskBody {
        task_id: entry.task_ref.clone(),
        summary: titles.get(&entry.task_ref).map_or_else(|| entry.task_ref.clone(), |t| t.title.clone()),
        reason: serde_json::to_value(entry.reason).expect("serializable reason").as_str().expect("reason string").to_owned(),
        deferred_by_task_refs: entry.deferred_by_task_refs.clone(),
        affected_dependent_task_refs: entry.affected_dependent_task_refs.clone(),
        explanation: entry.user_facing_summary.clone(),
        safe_alternatives: entry.safe_alternatives.clone().into_iter().map(Into::into).collect(),
    }).collect::<Vec<_>>();
    let selected_index = candidates.iter().position(|candidate| candidate.rank == 1);
    let canonical_plan_id = UbuId::new(ObjectType::Plan).to_string();
    let (plan, selected_candidate, alternatives, legitimization, risk_report, plan_quality) =
        match selected_index {
            Some(selected_index) => {
                let selected = candidates.remove(selected_index);
                let full_legitimization = ubu_planning_core::legitimization::full_legitimize(
                    &selected.schedule,
                    kernel_request.affect_profile.as_ref(),
                    kernel_request.affect_observation.as_ref(),
                );
                let legitimization = Some(legitimization_report_body(
                    full_legitimization.report,
                    planning_request.affect_warning.clone(),
                ));
                let selected_candidate =
                    kernel_candidate_body(&selected, &titles, &planning_request, &direct)?;
                let alternatives = candidates
                    .iter()
                    .map(|candidate| kernel_candidate_body(candidate, &titles, &planning_request, &direct))
                    .collect::<Result<Vec<_>>>()?;
                let (risk_report, plan_quality) = planning_analysis::analyze(
                    state.inner().store.pool(),
                    PlanningAnalysisInput {
                        plan_ref: &canonical_plan_id,
                        selected_candidate: Some(&selected_candidate),
                        legitimization: legitimization.as_ref(),
                        diagnostics: &diagnostics,
                        request: &planning_request,
                    },
                )
                .await?;
                let stored = persist_kernel_plan(
                    &state,
                    &canonical_plan_id,
                    &planning_request.request_id,
                    &selected.schedule,
                    &planning_request,
                    PersistPlanMetadata {
                        legitimization: legitimization.clone(),
                        selected_candidate: Some(selected_candidate.clone()),
                        alternatives: alternatives.clone(),
                        supersedes_plan_id: None,
                        risk_report: Some(risk_report.clone()),
                        human_complete_plan_quality: Some(plan_quality.clone()),
                    },
                    Vec::new(),
                    &direct,
                )
                .await?;
                if risk_report.findings.iter().any(|finding| finding.blocking) {
                    raise_blocking_recalculation(&state, &stored.id, &risk_report).await?;
                }
                (
                    Some(stored),
                    Some(selected_candidate),
                    alternatives,
                    legitimization,
                    Some(risk_report),
                    Some(plan_quality),
                )
            }
            None => {
                if !candidates.is_empty() {
                    diagnostics.push(DiagnosticBody {
                        code: "missing_rank_one_candidate".to_owned(),
                        message: "planning kernel returned candidates without a rank-1 selection"
                            .to_owned(),
                    });
                }
                let (risk_report, _plan_quality) = planning_analysis::analyze(
                    state.inner().store.pool(),
                    PlanningAnalysisInput {
                        plan_ref: &planning_request.request_id,
                        selected_candidate: None,
                        legitimization: None,
                        diagnostics: &diagnostics,
                        request: &planning_request,
                    },
                )
                .await?;
                if risk_report.findings.iter().any(|finding| finding.blocking) {
                    raise_blocking_recalculation(
                        &state,
                        &planning_request.request_id,
                        &risk_report,
                    )
                    .await?;
                }
                (None, None, Vec::new(), None, Some(risk_report), None)
            }
        };

    Ok(PlanningResponseBody {
        status: if plan.is_none() { "rejected" } else if unplaced_tasks.is_empty() { "ok" } else { "partial" }.into(),
        unplaced_tasks,
        task_priorities,
        schema_version: PLANNING_SCHEMA_VERSION.to_owned(),
        request_id: planning_request.request_id,
        plan,
        selected_candidate,
        alternatives,
        legitimization,
        diagnostics,
        blocked_tasks,
        invalid_tasks,
        risk_report,
        human_complete_plan_quality: plan_quality,
    })
}

pub async fn current_calendar(state: AppState) -> Result<CalendarResponse> {
    let pool = state.inner().store.pool();
    let row = sqlx::query(
        "SELECT payload_json FROM plans
        WHERE status = ?
        ORDER BY created_at DESC
        LIMIT 1",
    )
    .bind("admitted")
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some(row) = row else {
        return Ok(CalendarResponse {
            plan_id: None,
            steps: Vec::new(),
            display_probability: None,
            probability_interval_low: None,
            probability_interval_high: None,
            robustness_score: None,
            probability_quality: ProbabilityQualityBody::NotEstimated,
            legitimization: None,
            selected_candidate: None,
            alternatives: Vec::new(),
            stale: false,
            risk_report: None,
            human_complete_plan_quality: None,
        });
    };

    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let plan = canonical_plan_from_payload(&payload_json)?;

    let selected = plan.selected_candidate.as_ref();
    let stale = plan
        .risk_report
        .as_ref()
        .is_some_and(|report| report.findings.iter().any(|finding| finding.blocking));
    Ok(CalendarResponse {
        plan_id: Some(plan.id),
        steps: plan.steps,
        display_probability: selected.and_then(|candidate| candidate.display_probability),
        probability_interval_low: selected.and_then(|candidate| candidate.probability_interval_low),
        probability_interval_high: selected
            .and_then(|candidate| candidate.probability_interval_high),
        robustness_score: selected.map(|candidate| candidate.robustness_score),
        probability_quality: selected
            .map(|candidate| candidate.probability_quality)
            .unwrap_or(ProbabilityQualityBody::NotEstimated),
        legitimization: plan.legitimization,
        selected_candidate: plan.selected_candidate,
        alternatives: plan.alternatives,
        stale,
        risk_report: plan.risk_report,
        human_complete_plan_quality: plan.human_complete_plan_quality,
    })
}

pub async fn build_request_from_store(state: &AppState) -> Result<PlanningRequestBody> {
    Ok(
        build_request_from_store_with_context(state, PlanningModeBody::FreshGeneration, None, &[], None)
            .await?
            .request,
    )
}

pub async fn build_repair_request_from_store(
    state: &AppState,
    prior_plan: &PlanBody,
    repair_scope: RepairScopeBody,
    observed_divergence_refs: Vec<String>,
    frozen_task_ids: &[String],
) -> Result<StorePlanningRequest> {
    build_request_from_store_with_context(
        state,
        PlanningModeBody::Repair,
        Some(RepairContextBody {
            prior_plan_id: prior_plan.id.clone(),
            last_legitimate_plan_ref: prior_plan.id.clone(),
            observed_divergence_refs,
            repair_scope,
        }),
        frozen_task_ids,
        None,
    )
    .await
}

pub async fn latest_admitted_plan(state: &AppState) -> Result<Option<PlanBody>> {
    let row = sqlx::query(
        "SELECT payload_json FROM plans
        WHERE status = ?
        ORDER BY created_at DESC
        LIMIT 1",
    )
    .bind("admitted")
    .fetch_optional(state.inner().store.pool())
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    row.map(|row| -> Result<PlanBody> {
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        canonical_plan_from_payload(&payload_json)
    })
    .transpose()
}

pub async fn persist_repair_plan(
    state: &AppState,
    context: &StorePlanningRequest,
    repaired_plan: &KernelPlan,
    prior_plan: &PlanBody,
    frozen_steps: Vec<ScheduledTaskBody>,
) -> Result<PlanBody> {
    let request = &context.request;
    let plan_id = UbuId::new(ObjectType::Plan).to_string();
    persist_kernel_plan(
        state,
        &plan_id,
        &request.request_id,
        repaired_plan,
        request,
        PersistPlanMetadata {
            legitimization: None,
            selected_candidate: None,
            alternatives: Vec::new(),
            supersedes_plan_id: Some(prior_plan.id.clone()),
            risk_report: None,
            human_complete_plan_quality: None,
        },
        frozen_steps,
        &DirectPlacements { non_capacity: &context.non_capacity_tasks, covered: &context.covered_static_tasks, carriers: &context.carrier_windows },
    )
    .await
}

pub async fn supersede_plan(state: &AppState, prior_plan_id: &str) -> Result<()> {
    let pool = state.inner().store.pool();
    let row = sqlx::query("SELECT payload_json FROM plans WHERE id = ?")
        .bind(prior_plan_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut payload: Value = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize plan: {e}")))?;
    payload["status"] = json!("superseded");
    let updated_payload = serde_json::to_string(&payload)
        .map_err(|e| AppError::Internal(format!("failed to serialize plan: {e}")))?;

    sqlx::query("UPDATE plans SET status = ?, payload_json = ? WHERE id = ?")
        .bind("superseded")
        .bind(updated_payload)
        .bind(prior_plan_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

pub fn frozen_steps_for_plan(
    prior_plan: &PlanBody,
    frozen_task_ids: &HashSet<String>,
) -> Vec<ScheduledTaskBody> {
    prior_plan
        .steps
        .iter()
        .filter(|step| frozen_task_ids.contains(&step.task_id))
        .cloned()
        .collect()
}

pub fn kernel_plan_body(plan: KernelPlan) -> Result<PlanBody> {
    let created_at = UbuTimestamp::now_utc().to_string();
    Ok(PlanBody {
        id: plan.plan_id,
        status: format!("{:?}", plan.status).to_ascii_lowercase(),
        steps: plan
            .steps
            .into_iter()
            .enumerate()
            .map(|(index, task)| Ok(ScheduledTaskBody {
                index: index as u32,
                task_id: task.task_id.clone(),
                summary: task.task_id,
                start: task.start,
                end: task.end,
                start_at: crate::planning_time::timestamp_at(task.start)?,
                end_at: crate::planning_time::timestamp_at(task.end)?,
                depends_on: task.depends_on,
                static_anchor: task.static_anchor,
                occupies_capacity: true,
                category_tag: None,
                gcal_color_id: None,
                placement_authority: if task.static_anchor {
                    "user_override".to_owned()
                } else {
                    "planner".to_owned()
                },
            }))
            .collect::<Result<Vec<_>>>()?,
        created_at,
        supersedes_plan_id: None,
        legitimization: None,
        selected_candidate: None,
        alternatives: Vec::new(),
        risk_report: None,
        human_complete_plan_quality: None,
    })
}

fn validate_optional_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        None | Some(PLANNING_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
    }
}

async fn build_request_from_store_with_context(
    state: &AppState,
    mode: PlanningModeBody,
    repair_context: Option<RepairContextBody>,
    excluded_task_ids: &[String],
    explicit_horizon: Option<&PlanningHorizonBody>,
) -> Result<StorePlanningRequest> {
    let now = timestamp_seconds(&state.planning_now().to_string())?;
    let pre_repair_horizon = resolve_time_window(state, explicit_horizon, now).await?;
    let routine_context = super::routine_service::materialize(state, &pre_repair_horizon, now).await?;
    let pool = state.inner().store.pool();
    let tasks = queries::query_active_tasks(pool)
        .await
        .map_err(AppError::from)?;

    let static_windows = stored_static_windows(pool).await?;
    let excluded: HashSet<_> = excluded_task_ids.iter().cloned().collect();
    let task_rows = tasks
        .into_iter()
        .filter(|record| !excluded.contains(&record.id))
        .map(|record| {
            let payload = serde_json::from_str::<Value>(&record.payload_json)
                .map_err(|e| AppError::Internal(format!("failed to deserialize task: {e}")))?;
            Ok(TaskRow {
                id: record.id,
                payload,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let TaskPreconditionPartition {
        eligible: task_rows,
        blocked: blocked_tasks,
        invalid: invalid_tasks,
    } = partition_tasks_by_preconditions(pool, task_rows, crate::instance_mode::MVP_INSTANCE_MODE)
        .await?;

    // Resolve H using the existing fallback before applying participation rules.
    let mut task_bodies = task_rows.iter().map(|task| TaskSpecBody {
        mandatory: false,
                value: 1.0,        id: task.id.clone(), duration: duration_seconds(&task.payload),
        duration_estimate: None, correlation_groups: Vec::new(),
        depends_on: Vec::new(), window: None, static_anchor: None,
    }).collect::<Vec<_>>();

    let mut time_window = pre_repair_horizon.clone();
    if repair_context.is_some() {
        time_window.start = time_window.start.max(now);
        if let Some(prior_plan) = latest_admitted_plan(state).await? {
            if let Some(max_frozen_end) = prior_plan.steps.iter()
                .filter(|step| excluded.contains(&step.task_id)).map(|step| step.end).max() {
                time_window.start = time_window.start.max(max_frozen_end);
            }
        }
        if time_window.end <= time_window.start {
            let remaining_duration = task_bodies.iter().map(|task| task.duration)
                .sum::<u64>().max(DEFAULT_TASK_DURATION_SECONDS);
            time_window.end = time_window.start.saturating_add(remaining_duration);
        }
    }

    let horizon = time_window.clone();
    let mandatory: HashSet<_> = task_rows.iter().filter(|t| t.payload.get("occurrence").is_some_and(|v| !v.is_null())).map(|t| t.id.clone()).collect();
    let mut diagnostics = routine_context.diagnostics;
    let mut non_capacity_tasks = Vec::new();
    let mut absent_static_windows = HashMap::new();
    task_bodies.clear();
    let mut participating = Vec::new();
    for task in &task_rows {
        let capacity = task.payload.get("occupies_capacity").and_then(Value::as_bool).unwrap_or(true);
        let window = static_windows.get(&task.id);
        if let Some(window) = window {
            let outside_horizon = window.start >= horizon.end || window.end <= horizon.start;
            if !capacity || outside_horizon {
                absent_static_windows.insert(task.id.clone(), window.clone());
            }
            if outside_horizon { continue; }
            participating.push((task, window, capacity));
            let spec = TaskSpecBody {
                mandatory: mandatory.contains(&task.id),
                value: 1.0,                id: task.id.clone(), duration: window.end - window.start,
                duration_estimate: None, correlation_groups: Vec::new(),
                depends_on: dependency_ids(&task.payload), window: Some(window.clone()),
                static_anchor: Some(StaticAnchorBody { start: window.start }),
            };
            if capacity {
                time_window.start = time_window.start.min(window.start);
                time_window.end = time_window.end.max(window.end);
                task_bodies.push(spec);
            } else {
                non_capacity_tasks.push(spec);
            }
        } else if capacity || mandatory.contains(&task.id) {
            let mut window = horizon.clone();
            if let Some(range) = task.payload.get("allowed_time_range") {
                let range: AllowedTimeRange = serde_json::from_value(range.clone())
                    .map_err(|e| AppError::Internal(format!("failed to deserialize Task range: {e}")))?;
                window.start = window.start.max(timestamp_seconds(&range.earliest_start.to_string())?);
                let end = timestamp_seconds(&range.latest_finish.to_string())?;
                if mandatory.contains(&task.id) {
                    if timestamp_seconds(&range.earliest_start.to_string())? >= pre_repair_horizon.end || end <= pre_repair_horizon.start { continue; }
                    window.end = end;
                    window.start = window.start.max(routine_context.realized_floors.get(&task.id).copied().unwrap_or(0));
                } else { window.end = window.end.min(end); }
            }
            task_bodies.push(TaskSpecBody {
                mandatory: mandatory.contains(&task.id),
                value: 1.0,                id: task.id.clone(), duration: duration_seconds(&task.payload),
                duration_estimate: task_duration_estimate(&task.payload)?,
                correlation_groups: task_correlation_groups(&task.payload)?,
                depends_on: dependency_ids(&task.payload), window: Some(window),
                static_anchor: None,
            });
        } else {
            diagnostics.push(DiagnosticBody { code: "non_capacity_dynamic_task_unsupported".into(),
                message: format!("Dynamic non-capacity Task `{}` is unsupported", task.id) });
        }
    }
    // One diagnostic per pair, even if both occupancy and precedence conflict.
    // Clause (b) applies to all Static prerequisites, including non-capacity ones.
    let mut conflicts = BTreeSet::new();
    let mut dropped_edges = HashSet::new();
    for (i, (task, window, capacity)) in participating.iter().enumerate() {
        for (other, other_window, other_capacity) in participating.iter().skip(i + 1) {
            if *capacity && *other_capacity && window.start < other_window.end && window.end > other_window.start {
                if !mandatory.contains(&task.id) && !mandatory.contains(&other.id) {
                    add_static_conflict(&mut conflicts, &task.id, window, &other.id, other_window);
                }
            }
        }
        for dependency in dependency_ids(&task.payload) {
            if let Some(other_window) = static_windows.get(&dependency) {
                if other_window.end > window.start {
                    if mandatory.contains(&task.id) && mandatory.contains(&dependency) {
                        dropped_edges.insert((task.id.clone(), dependency.clone()));
                        diagnostics.push(DiagnosticBody { code: "routine_occurrence_edge_dropped".into(), message: format!("Routine occurrence `{}` has a stale Static edge to `{dependency}`; both retain their fixed placement",task.id) });
                    } else { add_static_conflict(&mut conflicts, &task.id, window, &dependency, other_window); }
                }
            }
        }
    }
    diagnostics.extend(conflicts.into_iter().map(|(_, first, second)| DiagnosticBody {
        code: "static_task_collision".into(),
        message: format!("Static Tasks `{first}` and `{second}` have conflicting fixed placements or dependencies"),
    }));
    for task in task_bodies.iter_mut().chain(non_capacity_tasks.iter_mut()) {
        task.depends_on.retain(|parent| !dropped_edges.contains(&(task.id.clone(), parent.clone())));
    }
    let (covered_static_tasks, carrier_windows) = committed_clusters(&mut task_bodies, &mandatory, &mut diagnostics);
    for task in &covered_static_tasks { absent_static_windows.insert(task.id.clone(), task.window.clone().unwrap()); }
    let fixed: Vec<_> = task_bodies.iter().filter(|t| t.static_anchor.is_some()).filter_map(|t|t.window.clone()).collect();
    // Keep original dependencies through every exclusion, including the fixpoint.
    let planned_ids: HashSet<_> = task_bodies.iter().map(|task| task.id.clone()).collect();
    let mut removed = HashSet::new();
    task_bodies.retain_mut(|task| {
        if task.static_anchor.is_some() { return true; }
        let window = task.window.as_mut().expect("Dynamic Tasks have a window");
        let mut dependency_outside_horizon = false;
        for dependency in &task.depends_on {
            if !planned_ids.contains(dependency) {
                if let Some(prerequisite) = absent_static_windows.get(dependency) {
                    window.start = window.start.max(prerequisite.end);
                    dependency_outside_horizon |= prerequisite.end >= horizon.end;
                }
            }
        }
        // Static-prerequisite push precedes the now floor, and its diagnostic
        // takes precedence over an empty/short occupancy window.
        window.start = window.start.max(now);
        let code = if dependency_outside_horizon {
            Some("dependency_outside_horizon")
        } else if window.end <= window.start
            || window.end - window.start < task_duration_model(task).placement_seconds() {
            Some("task_unplaceable")
        } else if mandatory.contains(&task.id) && !has_free_gap(task.window.as_ref().unwrap(), &fixed, task_duration_model(task).placement_seconds()) {
            Some("routine_no_free_time")
        } else { None };
        if let Some(code) = code {
            removed.insert(task.id.clone());
            let reason = if dependency_outside_horizon { "a Static prerequisite ends outside the horizon" }
                else if code == "routine_no_free_time" { "fixed commitments leave no free time in its allowed range" }
                else { "it cannot fit its allowed occupancy window at placement duration" };
            diagnostics.push(exclusion_diagnostic(&task.id, mandatory.contains(&task.id), code, reason));
            return false;
        }
        true
    });
    loop {
        let before = removed.len();
        task_bodies.retain(|task| {
            if task.static_anchor.is_none() && task.depends_on.iter().any(|id| removed.contains(id)) {
                removed.insert(task.id.clone());
                diagnostics.push(exclusion_diagnostic(&task.id, mandatory.contains(&task.id), "prerequisite_unplaceable", "it depends on an unplaceable Task"));
                false
            } else { true }
        });
        if removed.len() == before { break; }
    }
    // Only now drop edges to unplanned Tasks. Kept Statics break propagation;
    // lifecycle, precondition and frozen exclusions retain their old behavior.
    let planned_ids: HashSet<_> = task_bodies.iter().map(|task| task.id.clone()).collect();
    for task in &mut task_bodies {
        task.depends_on.retain(|dependency| planned_ids.contains(dependency));
    }

    let preferences = active_task_preferences(pool).await?;
    for task in &task_bodies { if mandatory.contains(&task.id) && task.static_anchor.is_none() { time_window.end = time_window.end.max(task.window.as_ref().unwrap().end); } }
    let eligible = task_bodies.iter().filter(|task| !mandatory.contains(&task.id)).map(|task| task.id.clone()).collect::<Vec<_>>();
    let priorities = super::task_priority::layer_preferences(&eligible, &preferences);
    let priority_order = priorities.order_keys();
    let values: HashMap<_, _> = priorities.tasks.iter().map(|task| (task.task_id.as_str(), task.value)).collect();
    for task in &mut task_bodies { task.value = if mandatory.contains(&task.id) { 0.0 } else { values[task.id.as_str()] }; }
    diagnostics.extend(priorities.cycles.iter().map(|members| DiagnosticBody {
        code: "preference_cycle".into(),
        message: format!("Preference cycle among Tasks [{}]; please resolve this high-priority consistency error", members.join(", ")),
    }));
    let mut deadlines = HashMap::new();
    for row in &task_rows {
        if let Some(range) = row.payload.get("allowed_time_range") {
            let range: AllowedTimeRange = serde_json::from_value(range.clone())
                .map_err(|e| AppError::Internal(format!("failed to deserialize Task range: {e}")))?;
            deadlines.insert(row.id.clone(), timestamp_seconds(&range.latest_finish.to_string())?);
        }
    }
    let task_graph = if has_static_conflicts(&diagnostics) {
        // Conflicts must return all diagnostics even if their dependency graph cycles.
        TaskGraphBody { topological_order: Vec::new(), edges: Vec::new() }
    } else { task_graph(&task_bodies, &priority_order, &deadlines, &mandatory)? };
    let request_id = UbuId::new(ObjectType::Plan).to_string();
    let rng_seed = stable_seed(&request_id, &time_window, &task_graph.topological_order);
    let affect_profile = build_affect_profile(pool).await?;
    let affect_resolution = resolve_affect_observation(pool, &affect_profile, &time_window).await?;

    Ok(StorePlanningRequest {
        task_priorities: priorities.tasks,
        request: PlanningRequestBody {
            schema_version: Some(PLANNING_SCHEMA_VERSION.to_owned()),
            request_id,
            mode,
            rng_seed: Some(rng_seed),
            compute_budget: ComputeBudgetBody::default(),
            strict_validation: false,
            time_window: Some(time_window),
            task_graph: Some(task_graph),
            repair_context,
            affect_profile: Some(affect_resolution.profile),
            affect_observation: Some(affect_resolution.observation),
            affect_warning: affect_resolution.warning,
            scoring_policy: ScoringPolicyBody::default(),
            tasks: task_bodies,
        },
        blocked_tasks,
        invalid_tasks,
        diagnostics,
        non_capacity_tasks,
        covered_static_tasks,
        carrier_windows,
    })
}

async fn persist_kernel_plan(
    state: &AppState,
    plan_id: &str,
    request_id: &str,
    kernel_plan: &KernelPlan,
    request: &PlanningRequestBody,
    metadata: PersistPlanMetadata,
    frozen_steps: Vec<ScheduledTaskBody>,
    direct: &DirectPlacements<'_>,
) -> Result<PlanBody> {
    let now = UbuTimestamp::now_utc().to_string();
    let titles = task_titles(state.inner().store.pool(), &state.inner().category_palette).await?;
    let steps = merge_steps(frozen_steps, kernel_plan.steps.iter()
        .map(|task| restored_task_body(task, &titles, request, direct.carriers))
        .collect::<Result<Vec<_>>>()?.into_iter()
        .chain(direct_static_steps(direct.non_capacity, &titles, false)?)
        .chain(direct_static_steps(direct.covered, &titles, true)?));

    let plan = PlanBody {
        id: plan_id.to_owned(),
        status: "admitted".to_owned(),
        steps,
        created_at: now.clone(),
        supersedes_plan_id: metadata.supersedes_plan_id,
        legitimization: metadata.legitimization,
        selected_candidate: metadata.selected_candidate,
        alternatives: metadata.alternatives,
        risk_report: metadata.risk_report,
        human_complete_plan_quality: metadata.human_complete_plan_quality,
    };
    validate_canonical_plan(&plan)?;

    let pool = state.inner().store.pool();
    queries::store_plan(
        pool,
        NewPlanRecord {
            id: plan_id.to_owned(),
            request_id: request_id.to_owned(),
            status: "admitted".to_owned(),
            payload: serde_json::to_value(&plan).map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    Ok(plan)
}

struct PersistPlanMetadata {
    legitimization: Option<LegitimizationReportBody>,
    selected_candidate: Option<PlanCandidateBody>,
    alternatives: Vec<PlanCandidateBody>,
    supersedes_plan_id: Option<String>,
    risk_report: Option<crate::api::reports::RiskReportResponse>,
    human_complete_plan_quality: Option<crate::api::reports::HumanCompletePlanQualityResponse>,
}

async fn raise_blocking_recalculation(
    state: &AppState,
    plan_ref: &str,
    report: &crate::api::reports::RiskReportResponse,
) -> Result<()> {
    let now = UbuTimestamp::now_utc().to_string();
    let categories = report
        .findings
        .iter()
        .filter(|finding| finding.blocking)
        .map(|finding| format!("{:?}", finding.category).to_ascii_lowercase())
        .collect::<Vec<_>>();
    let envelope = state.envelope_for(Default::default(), ubu_core::AuthoritySource::System,
        UbuTimestamp::parse(&now)?)?;
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "recalculation_requested".to_owned(),
            object_refs: json!([]),
            payload: json!({
                "triggered_at": now,
                "trigger_type": "worker_request",
                "note": format!(
                    "blocking derived planning risk for {plan_ref}: {}",
                    categories.join(", ")
                )
            }),
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

fn scheduled_task_body(
    task: &ScheduledTask,
    titles: &HashMap<String, TaskDisplay>,
    request: &PlanningRequestBody,
) -> Result<ScheduledTaskBody> {
    let placement_authority = request
        .tasks
        .iter()
        .find(|spec| spec.id == task.task_id)
        .and_then(|spec| spec.static_anchor.as_ref())
        .map(|_| "user_override")
        .unwrap_or("planner")
        .to_owned();

    Ok(ScheduledTaskBody {
        index: 0,
        task_id: task.task_id.clone(),
        summary: titles
            .get(&task.task_id)
            .map(|display| display.title.clone())
            .unwrap_or_else(|| task.task_id.clone()),
        start: task.start,
        end: task.end,
        start_at: crate::planning_time::timestamp_at(task.start)?,
        end_at: crate::planning_time::timestamp_at(task.end)?,
        depends_on: task.depends_on.clone(),
        static_anchor: task.static_anchor,
        placement_authority,
        occupies_capacity: true,
        category_tag: titles.get(&task.task_id).and_then(|display| display.category_tag.clone()),
        gcal_color_id: titles.get(&task.task_id).and_then(|display| display.gcal_color_id.clone()),
    })
}

/// Non-capacity steps do not enter the kernel: robustness, probability and
/// legitimization describe capacity work only. Risk and plan-quality analysis
/// sees these steps through the merged candidate bodies.
fn direct_static_steps(tasks: &[TaskSpecBody], titles: &HashMap<String, TaskDisplay>, occupies_capacity: bool) -> Result<Vec<ScheduledTaskBody>> {
    tasks.iter().map(|task| {
        let window = task.window.as_ref().expect("direct Static Task has a window");
        let display = titles.get(&task.id);
        Ok(ScheduledTaskBody {
            index: 0, task_id: task.id.clone(),
            summary: display.map(|d| d.title.clone()).unwrap_or_else(|| task.id.clone()),
            start_at: crate::planning_time::timestamp_at(window.start)?,
            end_at: crate::planning_time::timestamp_at(window.end)?,
            start: window.start, end: window.end, depends_on: task.depends_on.clone(),
            static_anchor: true, placement_authority: "user_override".into(), occupies_capacity,
            category_tag: display.and_then(|d| d.category_tag.clone()),
            gcal_color_id: display.and_then(|d| d.gcal_color_id.clone()),
        })
    }).collect()
}

fn merge_steps(mut frozen: Vec<ScheduledTaskBody>, others: impl IntoIterator<Item = ScheduledTaskBody>) -> Vec<ScheduledTaskBody> {
    let mut seen: HashSet<_> = frozen.iter().map(|step| step.task_id.clone()).collect();
    frozen.extend(others.into_iter().filter(|step| seen.insert(step.task_id.clone())));
    frozen.sort_by(|a, b| (a.start, a.end, &a.task_id).cmp(&(b.start, b.end, &b.task_id)));
    for (index, step) in frozen.iter_mut().enumerate() { step.index = index as u32; }
    frozen
}

pub fn add_empty_capacity_diagnostic(request: &PlanningRequestBody, diagnostics: &mut Vec<DiagnosticBody>) {
    if request.tasks.is_empty() {
        diagnostics.push(DiagnosticBody { code: "no_capacity_tasks_to_plan".into(),
            message: "No capacity Tasks remain for the planning kernel".into() });
    }
}

fn kernel_candidate_body(
    candidate: &PlanCandidate,
    titles: &HashMap<String, TaskDisplay>,
    request: &PlanningRequestBody,
    direct: &DirectPlacements<'_>,
) -> Result<PlanCandidateBody> {
    let steps = merge_steps(Vec::new(), candidate.schedule.steps.iter()
        .map(|task| restored_task_body(task, titles, request, direct.carriers))
        .collect::<Result<Vec<_>>>()?.into_iter()
        .chain(direct_static_steps(direct.non_capacity, titles, false)?)
        .chain(direct_static_steps(direct.covered, titles, true)?));

    Ok(PlanCandidateBody {
        candidate_id: candidate.candidate_id.clone(),
        rank: candidate.rank,
        candidate_role: candidate_role_body(candidate.candidate_role),
        steps,
        score_summary: score_summary_body(candidate.score_summary.clone()),
        feasibility_summary: feasibility_summary_body(candidate.feasibility_summary.clone()),
        semi_legitimization_summary: semi_legitimization_summary_body(
            candidate.semi_legitimization_summary.clone(),
        ),
        display_probability: candidate.probability_summary.display_probability,
        probability_interval_low: candidate.probability_summary.probability_interval_low,
        probability_interval_high: candidate.probability_summary.probability_interval_high,
        robustness_score: candidate.score_summary.robustness_score,
        probability_quality: probability_quality_body(
            candidate.probability_summary.probability_quality,
        ),
    })
}

fn canonical_plan_from_payload(payload_json: &str) -> Result<PlanBody> {
    match serde_json::from_str::<PlanBody>(payload_json) {
        Ok(plan) => {
            validate_canonical_plan(&plan)?;
            Ok(plan)
        }
        Err(_) => {
            let legacy: KernelPlan = serde_json::from_str(payload_json)
                .map_err(|e| AppError::Internal(format!("failed to deserialize plan: {e}")))?;
            kernel_plan_body(legacy)
        }
    }
}

fn validate_canonical_plan(plan: &PlanBody) -> Result<()> {
    if plan.id.trim().is_empty() {
        return Err(AppError::Internal("plan id is required".to_owned()));
    }
    match plan.status.as_str() {
        "candidate" | "admitted" | "rejected" | "superseded" => {}
        other => {
            return Err(AppError::Internal(format!(
                "plan has unsupported status `{other}`"
            )))
        }
    }
    for (expected, step) in plan.steps.iter().enumerate() {
        if step.index != expected as u32 {
            return Err(AppError::Internal(
                "plan step indexes must be contiguous".to_owned(),
            ));
        }
        if step.summary.trim().is_empty() {
            return Err(AppError::Internal(
                "plan step summary is required".to_owned(),
            ));
        }
        if step.start >= step.end {
            return Err(AppError::Internal(format!(
                "plan step `{}` has an impossible interval",
                step.task_id
            )));
        }
    }
    Ok(())
}

async fn resolve_time_window(
    state: &AppState,
    explicit: Option<&PlanningHorizonBody>,
    now: u64,
) -> Result<TimeWindowBody> {
    if let Some(explicit) = explicit {
        let invalid = || AppError::bad_request_diagnostic("invalid_horizon", "horizon requires RFC 3339 timestamps with start before end");
        let start = timestamp_seconds(&explicit.start).map_err(|_| invalid())?;
        let end = timestamp_seconds(&explicit.end).map_err(|_| invalid())?;
        if start >= end { return Err(invalid()); }
        return Ok(TimeWindowBody { start, end });
    }
    let pool = state.inner().store.pool();
    let row = sqlx::query(
        "SELECT window_start, window_end, payload_json FROM calendars
        ORDER BY created_at DESC
        LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    if let Some(row) = row {
        let window_start: String = row
            .try_get("window_start")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let window_end: String = row
            .try_get("window_end")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let start = timestamp_seconds(&window_start)?;
        let end = timestamp_seconds(&window_end)?;
        if start < end {
            return Ok(TimeWindowBody { start, end });
        }
    }

    Ok(TimeWindowBody {
        start: now,
        end: now.saturating_add(state.inner().planning_horizon_seconds),
    })
}

fn timestamp_seconds(value: &str) -> Result<u64> {
    let timestamp = UbuTimestamp::parse(value)
        .map_err(|e| AppError::bad_request_diagnostic("invalid_calendar_window", e.to_string()))?;
    let seconds = timestamp.inner().unix_timestamp();
    if seconds < 0 {
        return Err(AppError::bad_request_diagnostic(
            "invalid_calendar_window",
            "calendar windows before Unix epoch are not supported by the Phase A planner adapter",
        ));
    }
    Ok(seconds as u64)
}

fn task_graph(tasks: &[TaskSpecBody], priorities: &BTreeMap<String, u32>, deadlines: &HashMap<String, u64>, mandatory: &HashSet<String>) -> Result<TaskGraphBody> {
    let key = |id: &String| (if mandatory.contains(id) { 0 } else { 1 }, priorities.get(id).copied().unwrap_or(0), deadlines.get(id).copied().unwrap_or(u64::MAX), id.clone());
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges = Vec::new();
    for task in tasks {
        indegree.entry(task.id.clone()).or_insert(0);
        children.entry(task.id.clone()).or_default();
        for dependency in &task.depends_on {
            children
                .entry(dependency.clone())
                .or_default()
                .insert(task.id.clone());
            *indegree.entry(task.id.clone()).or_insert(0) += 1;
            edges.push(TaskGraphEdgeBody {
                before: dependency.clone(),
                after: task.id.clone(),
            });
        }
    }

    let mut ready: BTreeSet<(u8, u32, u64, String)> = indegree
        .iter()
        .filter_map(|(task_id, count)| (*count == 0).then(|| key(task_id)))
        .collect();
    let mut topological_order = Vec::with_capacity(tasks.len());
    while let Some((_, _, _, task_id)) = ready.pop_first() {
        topological_order.push(task_id.clone());
        if let Some(next_tasks) = children.get(&task_id) {
            for next in next_tasks {
                let count = indegree.get_mut(next).ok_or_else(|| {
                    AppError::Internal("dependency graph endpoint is missing".to_owned())
                })?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(key(next));
                }
            }
        }
    }

    if topological_order.len() != tasks.len() {
        return Err(AppError::bad_request_diagnostic(
            "cyclic_dependency_graph",
            "Task dependency graph must be acyclic",
        ));
    }

    Ok(TaskGraphBody {
        topological_order,
        edges,
    })
}

fn dependency_ids(payload: &Value) -> Vec<String> {
    ["blocked_by", "depends_on", "dependencies"]
        .iter()
        .filter_map(|field| payload.get(field).and_then(Value::as_array))
        .flat_map(|values| values.iter().filter_map(Value::as_str).map(str::to_owned))
        .collect()
}

fn duration_seconds(payload: &Value) -> u64 {
    if let Some(minutes) = payload.get("duration_minutes").and_then(Value::as_u64) {
        return minutes.saturating_mul(60).max(1);
    }
    if let Some(minutes) = payload.get("estimate_minutes").and_then(Value::as_u64) {
        return minutes.saturating_mul(60).max(1);
    }
    payload
        .get("estimate")
        .and_then(|estimate| estimate.get("seconds"))
        .and_then(Value::as_u64)
        .map(|seconds| seconds.max(1))
        .unwrap_or(DEFAULT_TASK_DURATION_SECONDS)
}

fn task_duration_estimate(payload: &Value) -> Result<Option<DurationEstimateBody>> {
    payload
        .get("duration_estimate")
        .cloned()
        .map(serde_json::from_value::<TaskDurationEstimate>)
        .transpose()
        .map(|estimate| estimate.map(duration_estimate_body))
        .map_err(|error| {
            AppError::Internal(format!(
                "failed to deserialize task duration_estimate: {error}"
            ))
        })
}

fn duration_estimate_body(estimate: TaskDurationEstimate) -> DurationEstimateBody {
    match estimate {
        TaskDurationEstimate::Fixed { seconds } => DurationEstimateBody::Fixed { seconds },
        TaskDurationEstimate::ShiftedLognormalP95 {
            min_seconds,
            mode_seconds,
            p95_seconds,
        } => DurationEstimateBody::ShiftedLognormalP95 {
            min_seconds,
            mode_seconds,
            p95_seconds,
        },
    }
}

fn task_correlation_groups(payload: &Value) -> Result<Vec<CorrelationGroupBody>> {
    payload
        .get("correlation_groups")
        .cloned()
        .map(serde_json::from_value::<Vec<TaskCorrelationGroup>>)
        .transpose()
        .map(|groups| {
            groups
                .unwrap_or_default()
                .into_iter()
                .map(|group| CorrelationGroupBody {
                    group: group.group,
                    strength: group.strength,
                })
                .collect()
        })
        .map_err(|error| {
            AppError::Internal(format!(
                "failed to deserialize task correlation_groups: {error}"
            ))
        })
}

fn task_duration_model(task: &TaskSpecBody) -> DurationModel {
match &task.duration_estimate {
            Some(DurationEstimateBody::Fixed { seconds }) => {
                DurationModel::Fixed { seconds: *seconds }
            }
            Some(DurationEstimateBody::ShiftedLognormalP95 {
                min_seconds,
                mode_seconds,
                p95_seconds,
            }) => DurationModel::ShiftedLognormalP95 {
                min_seconds: *min_seconds,
                mode_seconds: *mode_seconds,
                p95_seconds: *p95_seconds,
            },
            None => DurationModel::Fixed {
                seconds: task.duration,
            },
        }
}

fn validate_task_models(request: &PlanningRequestBody) -> Result<()> {
    for task in &request.tasks {
        let duration = task_duration_model(task);
        if let Err(message) = TaskSpec::new(task.id.clone(), duration.clone()) {
            return Err(AppError::bad_request_diagnostic(
                "invalid_duration_estimate",
                message,
            ));
        }
        if !task.value.is_finite() || task.value < 0.0 {
            return Err(AppError::bad_request_diagnostic("invalid_task_value", "Task value must be finite and non-negative"));
        }
        let kernel_task = TaskSpec {
            id: task.id.clone(),
            duration,
            correlation_groups: task
                .correlation_groups
                .iter()
                .map(|group| CorrelationGroup {
                    group: group.group.clone(),
                    strength: group.strength,
                })
                .collect(),
            value: task.value,
            priority: 1.0,
            mandatory: task.mandatory,
            depends_on: task.depends_on.clone(),
            window: None,
            static_anchor: None,
        };
        kernel_task.validate_contract().map_err(|message| {
            AppError::bad_request_diagnostic("invalid_correlation_groups", message)
        })?;
    }
    Ok(())
}

struct AffectResolution {
    profile: AffectProfileBody,
    observation: AffectObservationBody,
    warning: Option<String>,
}

async fn build_affect_profile(pool: &sqlx::SqlitePool) -> Result<AffectProfileBody> {
    let settings = active_settings(pool).await?;
    let energy_defaulted = !has_any_setting(
        &settings,
        &[
            "acceptable_energy_floor",
            "affect_energy_floor",
            "energy_floor",
        ],
    );
    let stress_defaulted = !has_any_setting(
        &settings,
        &[
            "tolerable_stress_ceiling",
            "affect_stress_ceiling",
            "stress_ceiling",
        ],
    );
    let intensity_defaulted = !has_any_setting(
        &settings,
        &[
            "tolerable_intensity_ceiling",
            "tolerable_mood_intensity_ceiling",
            "affect_mood_intensity_ceiling",
            "mood_intensity_ceiling",
        ],
    );

    let mut dimensions = BTreeMap::new();
    dimensions.insert(
        "energy".to_owned(),
        affect_tolerance(
            AffectDirectionBody::HigherIsBetter,
            setting_location(
                &settings,
                &[
                    "acceptable_energy_floor",
                    "affect_energy_floor",
                    "energy_floor",
                ],
                4.0,
            ),
            setting_freshness_seconds(&settings, "energy"),
        ),
    );
    dimensions.insert(
        "stress".to_owned(),
        affect_tolerance(
            AffectDirectionBody::LowerIsBetter,
            setting_location(
                &settings,
                &[
                    "tolerable_stress_ceiling",
                    "affect_stress_ceiling",
                    "stress_ceiling",
                ],
                7.0,
            ),
            setting_freshness_seconds(&settings, "stress"),
        ),
    );
    dimensions.insert(
        "mood_intensity".to_owned(),
        affect_tolerance(
            AffectDirectionBody::LowerIsBetter,
            setting_location(
                &settings,
                &[
                    "tolerable_intensity_ceiling",
                    "tolerable_mood_intensity_ceiling",
                    "affect_mood_intensity_ceiling",
                    "mood_intensity_ceiling",
                ],
                8.0,
            ),
            setting_freshness_seconds(&settings, "mood_intensity"),
        ),
    );

    let mut profile = AffectProfileBody {
        mode: AffectLegitimizationModeBody::Enforce,
        dimensions,
    };
    if energy_defaulted && stress_defaulted && intensity_defaulted {
        profile.mode = AffectLegitimizationModeBody::Enforce;
    }
    Ok(profile)
}

async fn active_task_preferences(pool: &sqlx::SqlitePool) -> Result<Vec<Preference>> {
    let rows = sqlx::query("SELECT payload_json FROM objects WHERE object_type = 'Preference' AND status = 'active'")
        .fetch_all(pool).await.map_err(|e| AppError::Internal(e.to_string()))?;
    rows.into_iter().map(|row| {
        let payload: String = row.try_get("payload_json").map_err(|e| AppError::Internal(e.to_string()))?;
        serde_json::from_str(&payload).map_err(|e| AppError::Internal(format!("failed to deserialize Preference: {e}")))
    }).collect()
}

async fn active_settings(pool: &sqlx::SqlitePool) -> Result<HashMap<String, Value>> {
    let rows = sqlx::query(
        "SELECT payload_json, version FROM objects
        WHERE object_type = ? AND status = ?
        ORDER BY updated_at DESC",
    )
    .bind(ObjectType::Setting.as_str())
    .bind("active")
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut settings = HashMap::new();
    for row in rows {
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let payload: Value = serde_json::from_str(&payload_json)
            .map_err(|e| AppError::Internal(format!("failed to deserialize setting: {e}")))?;
        let Some(name) = payload.get("name").and_then(Value::as_str) else {
            continue;
        };
        settings
            .entry(name.to_owned())
            .or_insert_with(|| payload.get("value").cloned().unwrap_or(Value::Null));
    }
    Ok(settings)
}

fn affect_tolerance(
    direction: AffectDirectionBody,
    location: f64,
    freshness_seconds: Option<u64>,
) -> AffectToleranceBody {
    AffectToleranceBody {
        direction,
        location,
        scale: DEFAULT_AFFECT_SCALE,
        threshold: DEFAULT_AFFECT_THRESHOLD,
        freshness_seconds,
    }
}

fn has_any_setting(settings: &HashMap<String, Value>, names: &[&str]) -> bool {
    names.iter().any(|name| settings.contains_key(*name))
}

fn setting_location(
    settings: &HashMap<String, Value>,
    names: &[&str],
    default_location: f64,
) -> f64 {
    names
        .iter()
        .find_map(|name| settings.get(*name).and_then(location_value))
        .unwrap_or(default_location)
}

fn location_value(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return finite_0_to_10(number);
    }
    let text = value.as_str()?.trim().to_ascii_lowercase();
    let mapped = match text.as_str() {
        "very_low" | "very low" => 2.0,
        "low" => 3.0,
        "medium" | "moderate" | "balanced" => 5.0,
        "high" => 7.0,
        "very_high" | "very high" => 8.0,
        _ => text.parse::<f64>().ok()?,
    };
    finite_0_to_10(mapped)
}

fn finite_0_to_10(value: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=10.0).contains(&value)).then_some(value)
}

fn setting_freshness_seconds(
    settings: &HashMap<String, Value>,
    dimension: &str,
) -> Option<u64> {
    let per_dimension = format!("{dimension}_freshness_seconds");
    settings
        .get(&per_dimension)
        .or_else(|| settings.get("affect_freshness_seconds"))
        .and_then(u64_value)
}

fn u64_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

async fn resolve_affect_observation(
    pool: &sqlx::SqlitePool,
    profile: &AffectProfileBody,
    time_window: &TimeWindowBody,
) -> Result<AffectResolution> {
    let profile_defaulted = affect_profile_uses_bootstrap_defaults(profile);
    let mut warnings = Vec::new();
    if profile_defaulted {
        warnings.push(
            "affect profile uses bootstrap default review priors; review calibration recommended"
                .to_owned(),
        );
    }

    let snapshot = latest_snapshot_affect(pool).await?;
    let fallback_reason = match snapshot {
        None => Some("missing affect observation"),
        Some(ref observation) if missing_profile_dimensions(profile, observation) => {
            Some("incomplete affect observation")
        }
        Some(ref observation) if stale_profile_dimensions(profile, observation, time_window) => {
            Some("stale affect observation")
        }
        Some(_) => None,
    };

    let mut resolved_profile = profile.clone();
    let observation = if let Some(reason) = fallback_reason {
        resolved_profile.mode = AffectLegitimizationModeBody::WarnOnly;
        warnings.push(format!(
            "{reason}; using bootstrap default profile observation in warn_only mode"
        ));
        bootstrap_affect_observation(profile, time_window.start)
    } else {
        snapshot.expect("snapshot exists when no fallback reason is present")
    };

    Ok(AffectResolution {
        profile: resolved_profile,
        observation,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    })
}

fn affect_profile_uses_bootstrap_defaults(profile: &AffectProfileBody) -> bool {
    matches!(
        profile.dimensions.get("energy"),
        Some(tolerance) if tolerance.location == 4.0
    ) && matches!(
        profile.dimensions.get("stress"),
        Some(tolerance) if tolerance.location == 7.0
    ) && matches!(
        profile.dimensions.get("mood_intensity"),
        Some(tolerance) if tolerance.location == 8.0
    )
}

async fn latest_snapshot_affect(pool: &sqlx::SqlitePool) -> Result<Option<AffectObservationBody>> {
    let row = sqlx::query(
        "SELECT payload_json, version FROM objects
        WHERE object_type = ? AND status = ?
        ORDER BY updated_at DESC
        LIMIT 1",
    )
    .bind(ObjectType::Snapshot.as_str())
    .bind("active")
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some(row) = row else {
        return Ok(None);
    };
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload: Value = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize snapshot: {e}")))?;
    Ok(snapshot_affect_observation(&payload)?)
}

fn snapshot_affect_observation(payload: &Value) -> Result<Option<AffectObservationBody>> {
    let Some(affect) = payload.get("affect") else {
        return Ok(None);
    };
    let source_kind = affect
        .get("source_kind")
        .and_then(Value::as_str)
        .unwrap_or("live_observation")
        .to_owned();
    let observed_at = affect
        .get("observed_at")
        .and_then(observed_at_seconds)
        .or_else(|| payload.get("captured_at").and_then(observed_at_seconds));
    let Some(observed_at) = observed_at else {
        return Ok(None);
    };
    let Some(dimensions_value) = affect.get("dimensions").and_then(Value::as_object) else {
        return Ok(None);
    };

    let mut dimensions = BTreeMap::new();
    for dimension in ["energy", "stress", "mood_intensity"] {
        let Some(value) = dimensions_value
            .get(dimension)
            .and_then(|dimension| dimension.get("value"))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        dimensions.insert(
            dimension.to_owned(),
            AffectObservationValueBody {
                value,
                observed_at,
                source_kind: source_kind.clone(),
            },
        );
    }
    if dimensions.is_empty() {
        return Ok(None);
    }
    Ok(Some(AffectObservationBody { dimensions }))
}

fn observed_at_seconds(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|timestamp| timestamp_seconds(timestamp).ok())
    })
}

fn missing_profile_dimensions(
    profile: &AffectProfileBody,
    observation: &AffectObservationBody,
) -> bool {
    profile
        .dimensions
        .keys()
        .any(|dimension| !observation.dimensions.contains_key(dimension))
}

fn stale_profile_dimensions(
    profile: &AffectProfileBody,
    observation: &AffectObservationBody,
    time_window: &TimeWindowBody,
) -> bool {
    profile.dimensions.iter().any(|(dimension, tolerance)| {
        let Some(freshness_seconds) = tolerance.freshness_seconds else {
            return false;
        };
        let Some(observed) = observation.dimensions.get(dimension) else {
            return false;
        };
        time_window
            .start
            .saturating_sub(observed.observed_at)
            > freshness_seconds
    })
}

fn bootstrap_affect_observation(
    profile: &AffectProfileBody,
    observed_at: u64,
) -> AffectObservationBody {
    AffectObservationBody {
        dimensions: profile
            .dimensions
            .iter()
            .map(|(dimension, tolerance)| {
                (
                    dimension.clone(),
                    AffectObservationValueBody {
                        value: tolerance.location,
                        observed_at,
                        source_kind: "bootstrap_default_profile".to_owned(),
                    },
                )
            })
            .collect(),
    }
}

struct TaskDisplay {
    title: String,
    category_tag: Option<String>,
    gcal_color_id: Option<String>,
}

async fn task_titles(pool: &sqlx::SqlitePool, palette: &crate::category_palette::CategoryPalette) -> Result<HashMap<String, TaskDisplay>> {
    let rows = sqlx::query("SELECT id, payload_json FROM objects WHERE object_type = ?")
        .bind(ObjectType::Task.as_str())
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut titles = HashMap::new();
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let payload: Value = serde_json::from_str(&payload_json)
            .map_err(|e| AppError::Internal(format!("failed to deserialize task: {e}")))?;
        let category_tag = payload.get("category_tag").and_then(Value::as_str).map(str::to_owned);
        let gcal_color_id = palette.color(category_tag.as_deref()).map(str::to_owned);
        titles.insert(id.clone(), TaskDisplay {
            title: payload.get("title").and_then(Value::as_str).unwrap_or(&id).to_owned(),
            category_tag,
            gcal_color_id,
        });
    }
    Ok(titles)
}

fn stable_seed(request_id: &str, time_window: &TimeWindowBody, order: &[String]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in request_id
        .bytes()
        .chain(time_window.start.to_le_bytes())
        .chain(time_window.end.to_le_bytes())
        .chain(order.iter().flat_map(|id| id.bytes()))
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn diagnostics_from_kernel(diagnostics: Vec<ubu_planning_core::Diagnostic>) -> Vec<DiagnosticBody> {
    diagnostics
        .into_iter()
        .map(|diagnostic| DiagnosticBody {
            code: format!("{:?}", diagnostic.code),
            message: diagnostic.message,
        })
        .collect()
}

fn precondition_diagnostics(
    blocked_tasks: &[BlockedTaskBody],
    invalid_tasks: &[InvalidTaskBody],
) -> Vec<DiagnosticBody> {
    let mut diagnostics = Vec::with_capacity(blocked_tasks.len() + invalid_tasks.len());
    diagnostics.extend(blocked_tasks.iter().map(|task| DiagnosticBody {
        code: "task_precondition_blocked".to_owned(),
        message: format!(
            "Task `{}` was excluded from planning because its UniverseState precondition evaluated false",
            task.task_id
        ),
    }));
    diagnostics.extend(invalid_tasks.iter().map(|task| DiagnosticBody {
        code: "task_precondition_invalid".to_owned(),
        message: format!(
            "Task `{}` was excluded from planning because its UniverseState precondition is invalid: {}",
            task.task_id, task.error
        ),
    }));
    diagnostics
}

#[derive(Debug)]
pub struct StorePlanningRequest {
    pub task_priorities: Vec<TaskPriorityBody>,
    pub request: PlanningRequestBody,
    pub blocked_tasks: Vec<BlockedTaskBody>,
    pub invalid_tasks: Vec<InvalidTaskBody>,
    pub diagnostics: Vec<DiagnosticBody>,
    pub non_capacity_tasks: Vec<TaskSpecBody>,
    pub covered_static_tasks: Vec<TaskSpecBody>,
    pub carrier_windows: HashMap<String, TimeWindowBody>,
}

pub fn has_static_conflicts(diagnostics: &[DiagnosticBody]) -> bool {
    diagnostics.iter().any(|d| d.code == "static_task_collision")
}

fn add_static_conflict(conflicts: &mut BTreeSet<(u64, String, String)>,
    first: &str, first_window: &TimeWindowBody, second: &str, second_window: &TimeWindowBody) {
    let (first, start, second) = if (first_window.start, first) <= (second_window.start, second) {
        (first, first_window.start, second)
    } else { (second, second_window.start, first) };
    conflicts.insert((start, first.to_owned(), second.to_owned()));
}

async fn stored_static_windows(pool: &sqlx::SqlitePool) -> Result<HashMap<String, TimeWindowBody>> {
    let rows = queries::query_active_tasks(pool).await.map_err(AppError::from)?;
    let mut windows = HashMap::new();
    for row in rows {
        let id = row.id;
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(format!("failed to deserialize Task `{id}`: {e}")))?;
        if let Some(value) = payload.get("static_window").filter(|value| !value.is_null()) {
            let window: StaticWindow = serde_json::from_value(value.clone())
                .map_err(|e| AppError::Internal(format!("invalid Static window for `{id}`: {e}")))?;
            let start = timestamp_seconds(&window.start.to_string())?;
            let end = timestamp_seconds(&window.end.to_string())?;
            windows.insert(id, TimeWindowBody { start, end });
        }
    }
    Ok(windows)
}

#[derive(Debug)]
struct TaskRow {
    id: String,
    payload: Value,
}

#[derive(Debug)]
struct TaskPreconditionPartition {
    eligible: Vec<TaskRow>,
    blocked: Vec<BlockedTaskBody>,
    invalid: Vec<InvalidTaskBody>,
}

async fn partition_tasks_by_preconditions(
    pool: &sqlx::SqlitePool,
    tasks: Vec<TaskRow>,
    mode: InstanceMode,
) -> Result<TaskPreconditionPartition> {
    let universe_state = current_universe_state(pool).await?;
    let mut eligible = Vec::with_capacity(tasks.len());
    let mut blocked = Vec::new();
    let mut invalid = Vec::new();

    for task in tasks {
        let Some(raw_precondition) = task.payload.get("preconditions").cloned() else {
            eligible.push(task);
            continue;
        };

        let precondition =
            match serde_json::from_value::<UniversePrecondition>(raw_precondition.clone()) {
                Ok(precondition) => precondition,
                Err(error) => {
                    invalid.push(InvalidTaskBody {
                        task_id: task.id,
                        precondition: raw_precondition,
                        error: error.to_string(),
                    });
                    continue;
                }
            };

        // An intrinsic-affect precondition under a mode that does not model
        // intrinsic affect (organization/worker) is invalid, not merely blocked.
        if let Err(error) = validate_precondition_for_mode(mode, &precondition) {
            invalid.push(InvalidTaskBody {
                task_id: task.id,
                precondition: precondition_value(&precondition),
                error: error.to_string(),
            });
            continue;
        }

        match evaluate_universe_precondition(&universe_state, &precondition) {
            Ok(true) => eligible.push(task),
            Ok(false) => blocked.push(BlockedTaskBody {
                task_id: task.id,
                precondition: precondition_value(&precondition),
            }),
            Err(error) => invalid.push(InvalidTaskBody {
                task_id: task.id,
                precondition: precondition_value(&precondition),
                error: precondition_error_message(error),
            }),
        }
    }

    Ok(TaskPreconditionPartition {
        eligible,
        blocked,
        invalid,
    })
}

/// Read the current (latest) `UniverseState`, synthesizing an empty one when
/// none has been recorded yet. Used by the precondition path, which evaluates
/// against an empty universe when nothing has been captured.
async fn current_universe_state(pool: &sqlx::SqlitePool) -> Result<UniverseState> {
    Ok(read_current_universe_state(pool).await?.map(|(state, _)| state).unwrap_or_else(|| {
        UniverseState::new(
            UbuTimestamp::now_utc(),
            "empty UniverseState synthesized by orchestrator",
        )
    }))
}

/// Read the current (latest) persisted `UniverseState`, or `None` when none has
/// been recorded yet. Shared with the effects path (Wiring-B), which must
/// distinguish "no current version" from an empty synthesized state because the
/// store's `persist_universe_state` updates an existing current version in place.
pub(crate) async fn read_current_universe_state(
    pool: &sqlx::SqlitePool,
) -> Result<Option<(UniverseState, i64)>> {
    let row = sqlx::query(
        "SELECT payload_json, version FROM objects
        WHERE object_type = ?
        ORDER BY updated_at DESC, created_at DESC
        LIMIT 1",
    )
    .bind(ObjectType::UniverseState.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let version: i64 = row.try_get("version").map_err(|e| AppError::Internal(e.to_string()))?;
    serde_json::from_str(&payload_json)
        .map(|state| Some((state, version)))
        .map_err(|e| AppError::Internal(format!("failed to deserialize UniverseState: {e}")))
}

fn precondition_value(precondition: &UniversePrecondition) -> Value {
    serde_json::to_value(precondition).unwrap_or_else(|_| json!({}))
}

fn precondition_error_message(error: UniversePreconditionError) -> String {
    error.to_string()
}

impl From<TimeWindowBody> for TimeWindow {
    fn from(value: TimeWindowBody) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<StaticAnchorBody> for ubu_planning_core::StaticAnchor {
    fn from(value: StaticAnchorBody) -> Self {
        Self { start: value.start }
    }
}

pub fn repair_kernel_request(request: &PlanningRequestBody) -> RepairRequest {
    let planning_request = PlanningRequest::from(request.clone());
    RepairRequest {
        schema_version: request.schema_version.clone(),
        request_id: request.request_id.clone(),
        candidate: KernelPlan {
            plan_id: "repair-candidate-empty".to_owned(),
            status: PlanStatus::Candidate,
            supersedes_plan_id: request
                .repair_context
                .as_ref()
                .map(|context| context.prior_plan_id.clone()),
            steps: Vec::new(),
        },
        rng_seed: planning_request.rng_seed,
        time_window: planning_request.time_window,
        tasks: planning_request.task_graph.tasks,
        topological_order: planning_request.task_graph.topological_order,
        repair_context: planning_request.repair_context,
        affect_profile: planning_request.affect_profile,
        affect_observation: planning_request.affect_observation,
    }
}

#[cfg(test)]
mod precondition_mode_tests {
    use super::*;
    use ubu_store::UbuStore;

    fn intrinsic_affect_precondition_task() -> TaskRow {
        TaskRow {
            id: UbuId::new(ObjectType::Task).to_string(),
            payload: json!({
                "preconditions": {
                    "target": "numeric_values.affect.energy",
                    "predicate": "equals",
                    "expected": 1.0
                }
            }),
        }
    }

    #[tokio::test]
    async fn organization_mode_marks_intrinsic_affect_precondition_invalid() {
        let store = UbuStore::in_memory().await.expect("store");
        let partition = partition_tasks_by_preconditions(
            store.pool(),
            vec![intrinsic_affect_precondition_task()],
            InstanceMode::OrganizationMode,
        )
        .await
        .expect("partition");
        assert_eq!(partition.invalid.len(), 1);
        assert!(partition.eligible.is_empty());
        assert!(partition.blocked.is_empty());
    }

    #[tokio::test]
    async fn user_mode_permits_intrinsic_affect_precondition() {
        // user_mode permits intrinsic affect; against an empty universe the
        // precondition evaluates false, so the task is blocked, not invalid.
        let store = UbuStore::in_memory().await.expect("store");
        let partition = partition_tasks_by_preconditions(
            store.pool(),
            vec![intrinsic_affect_precondition_task()],
            InstanceMode::UserMode,
        )
        .await
        .expect("partition");
        assert!(partition.invalid.is_empty());
        assert_eq!(partition.blocked.len(), 1);
    }
}

struct DirectPlacements<'a> {
    non_capacity: &'a [TaskSpecBody],
    covered: &'a [TaskSpecBody],
    carriers: &'a HashMap<String, TimeWindowBody>,
}
fn restored_task_body(task: &ScheduledTask, titles: &HashMap<String,TaskDisplay>, request: &PlanningRequestBody, carriers: &HashMap<String,TimeWindowBody>) -> Result<ScheduledTaskBody> {
    let mut step = scheduled_task_body(task,titles,request)?;
    if let Some(window) = carriers.get(&task.task_id) {
        step.start = window.start; step.end = window.end;
        step.start_at = crate::planning_time::timestamp_at(window.start)?;
        step.end_at = crate::planning_time::timestamp_at(window.end)?;
    }
    Ok(step)
}
fn exclusion_diagnostic(id: &str, mandatory: bool, code: &str, reason: &str) -> DiagnosticBody {
    if mandatory { DiagnosticBody { code: "mandatory_occurrence_unplaceable".into(), message: format!("Mandatory routine occurrence `{id}` was left out of the Plan: {reason}; do it late, skip it, or change other commitments") } }
    else { DiagnosticBody {code:code.into(), message:format!("Task `{id}` {reason}")} }
}
fn has_free_gap(window: &TimeWindowBody, fixed: &[TimeWindowBody], duration: u64) -> bool {
    let mut intervals: Vec<_> = fixed.iter().filter(|w| w.start < window.end && w.end > window.start).collect();
    intervals.sort_by_key(|w|(w.start,w.end));
    let mut cursor=window.start;
    for interval in intervals {
        if interval.start.saturating_sub(cursor)>=duration {return true;}
        cursor=cursor.max(interval.end);
    }
    window.end.saturating_sub(cursor)>=duration
}
/// Reserve each connected committed-time span once, retaining individual Calendar windows.
fn committed_clusters(tasks: &mut Vec<TaskSpecBody>, mandatory: &HashSet<String>, diagnostics: &mut Vec<DiagnosticBody>) -> (Vec<TaskSpecBody>,HashMap<String,TimeWindowBody>) {
    fn root(parents:&mut [usize], mut n:usize)->usize {
        while parents[n]!=n {parents[n]=parents[parents[n]];n=parents[n];} n
    }
    let mut parents:Vec<_>=(0..tasks.len()).collect();
    for i in 0..tasks.len() {
        if tasks[i].static_anchor.is_none() {continue;}
        for j in i+1..tasks.len() {
            if tasks[j].static_anchor.is_none() || (!mandatory.contains(&tasks[i].id) && !mandatory.contains(&tasks[j].id)) {continue;}
            let a=tasks[i].window.as_ref().unwrap();let b=tasks[j].window.as_ref().unwrap();
            if a.start<b.end && a.end>b.start {let x=root(&mut parents,i);let y=root(&mut parents,j);parents[x]=y;}
        }
    }
    let mut groups=BTreeMap::<usize,Vec<usize>>::new();
    for i in 0..tasks.len() {groups.entry(root(&mut parents,i)).or_default().push(i);}
    let mut covered=Vec::new();let mut carriers=HashMap::new();let mut removed=HashSet::new();let mut warnings=Vec::new();
    for mut group in groups.into_values().filter(|g|g.len()>1) {
        group.sort_by_key(|&i| {let w=tasks[i].window.as_ref().unwrap();(w.start,std::cmp::Reverse(w.end),tasks[i].id.clone())});
        let carrier=group[0];let members:HashSet<_>=group.iter().map(|&i|tasks[i].id.clone()).collect();
        let mut occurrences:Vec<_>=members.iter().filter(|id|mandatory.contains(*id)).cloned().collect();occurrences.sort();
        let commitment=group.iter().map(|&i|&tasks[i]).filter(|t|!mandatory.contains(&t.id)).min_by_key(|t|(t.window.as_ref().unwrap().start,t.id.clone()));
        if let Some(commitment)=commitment {
            for id in &occurrences {warnings.push(DiagnosticBody {code:"routine_occurrence_overlaps_commitment".into(),message:format!("Routine occurrence `{id}` shares its time with commitment `{}`; both stay on the Calendar and the whole span is busy",commitment.id)});}
        }
        if occurrences.len()>1 {warnings.push(DiagnosticBody {code:"routine_occurrences_overlap".into(),message:format!("Routine occurrences {} overlap; routines are not meant to overlap and their definitions need review",occurrences.iter().map(|id|format!("`{id}`")).collect::<Vec<_>>().join(", "))});}
        let start=tasks[carrier].window.as_ref().unwrap().start;
        let end=group.iter().map(|&i|tasks[i].window.as_ref().unwrap().end).max().unwrap();
        let dependencies:BTreeSet<_>=group.iter().flat_map(|&i|tasks[i].depends_on.iter()).filter(|id|!members.contains(*id)).cloned().collect();
        carriers.insert(tasks[carrier].id.clone(),tasks[carrier].window.clone().unwrap());
        for &i in &group[1..] {covered.push(tasks[i].clone());removed.insert(tasks[i].id.clone());}
        tasks[carrier].duration=end-start;tasks[carrier].window=Some(TimeWindowBody{start,end});tasks[carrier].static_anchor=Some(StaticAnchorBody{start});tasks[carrier].depends_on=dependencies.into_iter().collect();
    }
    tasks.retain(|t|!removed.contains(&t.id));
    warnings.sort_by(|a,b|(&a.code,&a.message).cmp(&(&b.code,&b.message)));diagnostics.extend(warnings);
    (covered,carriers)
}
