use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_planning_core::{
    Plan as KernelPlan, PlanCandidate, PlanStatus, PlanningRequest, RepairRequest, ScheduledTask,
    TimeWindow, PLANNING_SCHEMA_VERSION,
};
use ubu_store::models::plan_record::NewPlanRecord;
use ubu_store::queries;

use crate::adapters::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::api::calendar::CalendarResponse;
use crate::api::planning::{
    candidate_role_body, feasibility_summary_body, legitimization_report_body, score_summary_body,
    semi_legitimization_summary_body, AffectDirectionBody, AffectLegitimizationModeBody,
    AffectObservationBody, AffectObservationValueBody, AffectProfileBody, AffectToleranceBody,
    DiagnosticBody, GeneratePlanningRequest, LegitimizationReportBody, PlanBody, PlanCandidateBody,
    PlanningModeBody, PlanningRequestBody, PlanningResponseBody, RepairContextBody,
    RepairScopeBody, ScheduledTaskBody, ScoringPolicyBody, StaticAnchorBody, TaskGraphBody,
    TaskGraphEdgeBody, TaskSpecBody, TimeWindowBody,
};
use crate::errors::{AppError, Result};
use crate::state::AppState;

const DEFAULT_TASK_DURATION_MINUTES: u64 = 30;
const DEFAULT_AFFECT_SCALE: f64 = 1.5;
const DEFAULT_AFFECT_THRESHOLD: f64 = 0.5;

pub async fn generate(
    state: AppState,
    request: GeneratePlanningRequest,
) -> Result<PlanningResponseBody> {
    validate_optional_schema_version(request.schema_version.as_deref())?;
    let planning_request = match request.request {
        Some(body) => {
            validate_optional_schema_version(body.schema_version.as_deref())?;
            body
        }
        None => build_request_from_store(&state).await?,
    };

    let kernel_request = PlanningRequest::from(planning_request.clone());
    let adapter = CpuPlannerAdapter;
    let response = adapter.plan(kernel_request.clone());
    let mut diagnostics = diagnostics_from_kernel(response.diagnostics);
    let mut candidates = response.plan_candidates;
    let selected_index = candidates.iter().position(|candidate| candidate.rank == 1);
    let (plan, selected_candidate, alternatives, legitimization) = match selected_index {
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
            let titles = task_titles(state.inner().store.pool()).await?;
            let selected_candidate = kernel_candidate_body(&selected, &titles, &planning_request);
            let alternatives = candidates
                .iter()
                .map(|candidate| kernel_candidate_body(candidate, &titles, &planning_request))
                .collect::<Vec<_>>();
            let stored = persist_kernel_plan(
                &state,
                &planning_request.request_id,
                &selected.schedule,
                &planning_request,
                PersistPlanMetadata {
                    legitimization: legitimization.clone(),
                    selected_candidate: Some(selected_candidate.clone()),
                    alternatives: alternatives.clone(),
                    supersedes_plan_id: None,
                },
                Vec::new(),
            )
            .await?;
            (
                Some(stored),
                Some(selected_candidate),
                alternatives,
                legitimization,
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
            (None, None, Vec::new(), None)
        }
    };

    Ok(PlanningResponseBody {
        schema_version: response.schema_version,
        request_id: response.request_id,
        plan,
        selected_candidate,
        alternatives,
        legitimization,
        diagnostics,
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
            legitimization: None,
            selected_candidate: None,
            alternatives: Vec::new(),
        });
    };

    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let plan = canonical_plan_from_payload(&payload_json)?;

    Ok(CalendarResponse {
        plan_id: Some(plan.id),
        steps: plan.steps,
        legitimization: plan.legitimization,
        selected_candidate: plan.selected_candidate,
        alternatives: plan.alternatives,
    })
}

pub async fn build_request_from_store(state: &AppState) -> Result<PlanningRequestBody> {
    build_request_from_store_with_context(state, PlanningModeBody::FreshGeneration, None, &[]).await
}

pub async fn build_repair_request_from_store(
    state: &AppState,
    prior_plan: &PlanBody,
    repair_scope: RepairScopeBody,
    observed_divergence_refs: Vec<String>,
    frozen_task_ids: &[String],
) -> Result<PlanningRequestBody> {
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
    request: &PlanningRequestBody,
    repaired_plan: &KernelPlan,
    prior_plan: &PlanBody,
    frozen_steps: Vec<ScheduledTaskBody>,
) -> Result<PlanBody> {
    persist_kernel_plan(
        state,
        &request.request_id,
        repaired_plan,
        request,
        PersistPlanMetadata {
            legitimization: None,
            selected_candidate: None,
            alternatives: Vec::new(),
            supersedes_plan_id: Some(prior_plan.id.clone()),
        },
        frozen_steps,
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

pub fn kernel_plan_body(plan: KernelPlan) -> PlanBody {
    let created_at = UbuTimestamp::now_utc().to_string();
    PlanBody {
        id: plan.plan_id,
        status: format!("{:?}", plan.status).to_ascii_lowercase(),
        steps: plan
            .steps
            .into_iter()
            .enumerate()
            .map(|(index, task)| ScheduledTaskBody {
                index: index as u32,
                task_id: task.task_id.clone(),
                summary: task.task_id,
                start: task.start,
                end: task.end,
                depends_on: task.depends_on,
                static_anchor: task.static_anchor,
                placement_authority: if task.static_anchor {
                    "user_override".to_owned()
                } else {
                    "planner".to_owned()
                },
            })
            .collect(),
        created_at,
        supersedes_plan_id: None,
        legitimization: None,
        selected_candidate: None,
        alternatives: Vec::new(),
    }
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
) -> Result<PlanningRequestBody> {
    let pool = state.inner().store.pool();
    let tasks = queries::query_active_tasks(pool)
        .await
        .map_err(AppError::from)?;

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

    if task_rows.is_empty() {
        return Err(AppError::BadRequest(
            "import GitHub candidates before generating a plan".to_owned(),
        ));
    }

    let planned_task_ids: HashSet<String> = task_rows.iter().map(|task| task.id.clone()).collect();
    let mut task_bodies = Vec::with_capacity(task_rows.len());
    for task in &task_rows {
        let dependencies = dependency_ids(&task.payload)
            .into_iter()
            .filter(|dependency| planned_task_ids.contains(dependency))
            .collect::<Vec<_>>();
        task_bodies.push(TaskSpecBody {
            id: task.id.clone(),
            duration: duration_minutes(&task.payload),
            depends_on: dependencies,
            window: None,
            static_anchor: None,
        });
    }

    let mut time_window = resolve_time_window(pool, &task_bodies).await?;
    if repair_context.is_some() {
        if let Some(prior_plan) = latest_admitted_plan(state).await? {
            let frozen: HashSet<String> = excluded_task_ids.iter().cloned().collect();
            if let Some(max_frozen_end) = prior_plan
                .steps
                .iter()
                .filter(|step| frozen.contains(&step.task_id))
                .map(|step| step.end)
                .max()
            {
                time_window.start = time_window.start.max(max_frozen_end);
                if time_window.end <= time_window.start {
                    let remaining_duration = task_bodies
                        .iter()
                        .map(|task| task.duration)
                        .sum::<u64>()
                        .max(DEFAULT_TASK_DURATION_MINUTES);
                    time_window.end = time_window.start + remaining_duration;
                }
            }
        }
    }

    for task in &mut task_bodies {
        task.window = Some(time_window.clone());
    }

    let task_graph = task_graph(&task_bodies)?;
    let request_id = UbuId::new(ObjectType::Plan).to_string();
    let rng_seed = stable_seed(&request_id, &time_window, &task_graph.topological_order);
    let affect_profile = build_affect_profile(pool).await?;
    let affect_resolution = resolve_affect_observation(pool, &affect_profile, &time_window).await?;

    Ok(PlanningRequestBody {
        schema_version: Some(PLANNING_SCHEMA_VERSION.to_owned()),
        request_id,
        mode,
        rng_seed: Some(rng_seed),
        time_window: Some(time_window),
        task_graph: Some(task_graph),
        repair_context,
        affect_profile: Some(affect_resolution.profile),
        affect_observation: Some(affect_resolution.observation),
        affect_warning: affect_resolution.warning,
        scoring_policy: ScoringPolicyBody::default(),
        tasks: task_bodies,
    })
}

async fn persist_kernel_plan(
    state: &AppState,
    request_id: &str,
    kernel_plan: &KernelPlan,
    request: &PlanningRequestBody,
    metadata: PersistPlanMetadata,
    frozen_steps: Vec<ScheduledTaskBody>,
) -> Result<PlanBody> {
    let plan_id = UbuId::new(ObjectType::Plan).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let titles = task_titles(state.inner().store.pool()).await?;
    let mut steps = frozen_steps;
    let existing_task_ids: HashSet<String> =
        steps.iter().map(|step| step.task_id.clone()).collect();
    steps.extend(
        kernel_plan
            .steps
            .iter()
            .filter(|task| !existing_task_ids.contains(&task.task_id))
            .map(|task| scheduled_task_body(task, &titles, request)),
    );
    steps.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.end.cmp(&right.end))
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    for (index, step) in steps.iter_mut().enumerate() {
        step.index = index as u32;
    }

    let plan = PlanBody {
        id: plan_id.clone(),
        status: "admitted".to_owned(),
        steps,
        created_at: now.clone(),
        supersedes_plan_id: metadata.supersedes_plan_id,
        legitimization: metadata.legitimization,
        selected_candidate: metadata.selected_candidate,
        alternatives: metadata.alternatives,
    };
    validate_canonical_plan(&plan)?;

    let pool = state.inner().store.pool();
    queries::store_plan(
        pool,
        NewPlanRecord {
            id: plan_id,
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
}

fn scheduled_task_body(
    task: &ScheduledTask,
    titles: &HashMap<String, String>,
    request: &PlanningRequestBody,
) -> ScheduledTaskBody {
    let placement_authority = request
        .tasks
        .iter()
        .find(|spec| spec.id == task.task_id)
        .and_then(|spec| spec.static_anchor.as_ref())
        .map(|_| "user_override")
        .unwrap_or("planner")
        .to_owned();

    ScheduledTaskBody {
        index: 0,
        task_id: task.task_id.clone(),
        summary: titles
            .get(&task.task_id)
            .cloned()
            .unwrap_or_else(|| task.task_id.clone()),
        start: task.start,
        end: task.end,
        depends_on: task.depends_on.clone(),
        static_anchor: task.static_anchor,
        placement_authority,
    }
}

fn kernel_candidate_body(
    candidate: &PlanCandidate,
    titles: &HashMap<String, String>,
    request: &PlanningRequestBody,
) -> PlanCandidateBody {
    let mut steps = candidate
        .schedule
        .steps
        .iter()
        .map(|task| scheduled_task_body(task, titles, request))
        .collect::<Vec<_>>();
    steps.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.end.cmp(&right.end))
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    for (index, step) in steps.iter_mut().enumerate() {
        step.index = index as u32;
    }

    PlanCandidateBody {
        candidate_id: candidate.candidate_id.clone(),
        rank: candidate.rank,
        candidate_role: candidate_role_body(candidate.candidate_role),
        steps,
        score_summary: score_summary_body(candidate.score_summary.clone()),
        feasibility_summary: feasibility_summary_body(candidate.feasibility_summary.clone()),
        semi_legitimization_summary: semi_legitimization_summary_body(
            candidate.semi_legitimization_summary.clone(),
        ),
    }
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
            Ok(kernel_plan_body(legacy))
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
    pool: &sqlx::SqlitePool,
    tasks: &[TaskSpecBody],
) -> Result<TimeWindowBody> {
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
        let start = timestamp_minutes(&window_start)?;
        let end = timestamp_minutes(&window_end)?;
        if start < end {
            return Ok(TimeWindowBody { start, end });
        }
    }

    let total_duration = tasks
        .iter()
        .map(|task| task.duration)
        .sum::<u64>()
        .max(DEFAULT_TASK_DURATION_MINUTES);
    Ok(TimeWindowBody {
        start: 0,
        end: total_duration,
    })
}

fn timestamp_minutes(value: &str) -> Result<u64> {
    let timestamp = UbuTimestamp::parse(value)
        .map_err(|e| AppError::bad_request_diagnostic("invalid_calendar_window", e.to_string()))?;
    let seconds = timestamp.inner().unix_timestamp();
    if seconds < 0 {
        return Err(AppError::bad_request_diagnostic(
            "invalid_calendar_window",
            "calendar windows before Unix epoch are not supported by the Phase A planner adapter",
        ));
    }
    Ok((seconds as u64) / 60)
}

fn task_graph(tasks: &[TaskSpecBody]) -> Result<TaskGraphBody> {
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

    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter_map(|(task_id, count)| (*count == 0).then_some(task_id.clone()))
        .collect();
    let mut topological_order = Vec::with_capacity(tasks.len());
    while let Some(task_id) = ready.pop_first() {
        topological_order.push(task_id.clone());
        if let Some(next_tasks) = children.get(&task_id) {
            for next in next_tasks {
                let count = indegree.get_mut(next).ok_or_else(|| {
                    AppError::Internal("dependency graph endpoint is missing".to_owned())
                })?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(next.clone());
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

fn duration_minutes(payload: &Value) -> u64 {
    if let Some(minutes) = payload.get("duration_minutes").and_then(Value::as_u64) {
        return minutes.max(1);
    }
    if let Some(minutes) = payload.get("estimate_minutes").and_then(Value::as_u64) {
        return minutes.max(1);
    }
    payload
        .get("estimate")
        .and_then(|estimate| estimate.get("seconds"))
        .and_then(Value::as_u64)
        .map(|seconds| seconds.div_ceil(60).max(1))
        .unwrap_or(DEFAULT_TASK_DURATION_MINUTES)
}

struct AffectResolution {
    profile: AffectProfileBody,
    observation: AffectObservationBody,
    warning: Option<String>,
}

async fn build_affect_profile(pool: &sqlx::SqlitePool) -> Result<AffectProfileBody> {
    let preferences = active_preferences(pool).await?;
    let energy_defaulted = !has_any_preference(
        &preferences,
        &[
            "acceptable_energy_floor",
            "affect_energy_floor",
            "energy_floor",
        ],
    );
    let stress_defaulted = !has_any_preference(
        &preferences,
        &[
            "tolerable_stress_ceiling",
            "affect_stress_ceiling",
            "stress_ceiling",
        ],
    );
    let intensity_defaulted = !has_any_preference(
        &preferences,
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
            preference_location(
                &preferences,
                &[
                    "acceptable_energy_floor",
                    "affect_energy_floor",
                    "energy_floor",
                ],
                4.0,
            ),
            preference_freshness_seconds(&preferences, "energy"),
        ),
    );
    dimensions.insert(
        "stress".to_owned(),
        affect_tolerance(
            AffectDirectionBody::LowerIsBetter,
            preference_location(
                &preferences,
                &[
                    "tolerable_stress_ceiling",
                    "affect_stress_ceiling",
                    "stress_ceiling",
                ],
                7.0,
            ),
            preference_freshness_seconds(&preferences, "stress"),
        ),
    );
    dimensions.insert(
        "mood_intensity".to_owned(),
        affect_tolerance(
            AffectDirectionBody::LowerIsBetter,
            preference_location(
                &preferences,
                &[
                    "tolerable_intensity_ceiling",
                    "tolerable_mood_intensity_ceiling",
                    "affect_mood_intensity_ceiling",
                    "mood_intensity_ceiling",
                ],
                8.0,
            ),
            preference_freshness_seconds(&preferences, "mood_intensity"),
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

async fn active_preferences(pool: &sqlx::SqlitePool) -> Result<HashMap<String, Value>> {
    let rows = sqlx::query(
        "SELECT payload_json FROM objects
        WHERE object_type = ? AND status = ?
        ORDER BY updated_at DESC",
    )
    .bind(ObjectType::Preference.as_str())
    .bind("active")
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut preferences = HashMap::new();
    for row in rows {
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let payload: Value = serde_json::from_str(&payload_json)
            .map_err(|e| AppError::Internal(format!("failed to deserialize preference: {e}")))?;
        let Some(name) = payload.get("name").and_then(Value::as_str) else {
            continue;
        };
        preferences
            .entry(name.to_owned())
            .or_insert_with(|| payload.get("value").cloned().unwrap_or(Value::Null));
    }
    Ok(preferences)
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

fn has_any_preference(preferences: &HashMap<String, Value>, names: &[&str]) -> bool {
    names.iter().any(|name| preferences.contains_key(*name))
}

fn preference_location(
    preferences: &HashMap<String, Value>,
    names: &[&str],
    default_location: f64,
) -> f64 {
    names
        .iter()
        .find_map(|name| preferences.get(*name).and_then(location_value))
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

fn preference_freshness_seconds(
    preferences: &HashMap<String, Value>,
    dimension: &str,
) -> Option<u64> {
    let per_dimension = format!("{dimension}_freshness_seconds");
    preferences
        .get(&per_dimension)
        .or_else(|| preferences.get("affect_freshness_seconds"))
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
        "SELECT payload_json FROM objects
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
        .and_then(observed_at_minutes)
        .or_else(|| payload.get("captured_at").and_then(observed_at_minutes));
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

fn observed_at_minutes(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|timestamp| timestamp_minutes(timestamp).ok())
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
            .saturating_mul(60)
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

async fn task_titles(pool: &sqlx::SqlitePool) -> Result<HashMap<String, String>> {
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
        if let Some(title) = payload.get("title").and_then(Value::as_str) {
            titles.insert(id, title.to_owned());
        }
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

struct TaskRow {
    id: String,
    payload: Value,
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
