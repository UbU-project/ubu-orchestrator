use std::collections::HashMap;

use serde_json::Value;
use sqlx::Row;
use ubu_core::id_registry::ObjectType;
use ubu_core::UbuTimestamp;

use crate::api::planning::{
    DiagnosticBody, LegitimizationReportBody, PlanCandidateBody, PlanningRequestBody,
    ScheduledTaskBody,
};
use crate::api::reports::{
    CheckpointCoverage, FailurePattern, HumanCompletePlanQualityResponse, PostPlanStateDelta,
    RiskCategory, RiskFinding, RiskLevel, RiskReportResponse, StretchPressure,
};
use crate::errors::{AppError, Result};

const LITTLE_SLACK_SECONDS: u64 = 300;
const NEAR_AFFECT_MARGIN: f64 = 0.10;
const RECENT_LOG_LIMIT: i64 = 50;

pub struct PlanningAnalysisInput<'a> {
    pub plan_ref: &'a str,
    pub selected_candidate: Option<&'a PlanCandidateBody>,
    pub legitimization: Option<&'a LegitimizationReportBody>,
    pub diagnostics: &'a [DiagnosticBody],
    pub request: &'a PlanningRequestBody,
}

#[derive(Debug)]
struct StoreContext {
    tasks: HashMap<String, Value>,
    recent_logs: Vec<LogContext>,
    active_worker_count: usize,
}

#[derive(Debug)]
struct LogContext {
    event_type: String,
    payload: Value,
}

pub async fn analyze(
    pool: &sqlx::SqlitePool,
    input: PlanningAnalysisInput<'_>,
) -> Result<(RiskReportResponse, HumanCompletePlanQualityResponse)> {
    let context = load_store_context(pool).await?;
    Ok(derive_reports(input, &context))
}

fn derive_reports(
    input: PlanningAnalysisInput<'_>,
    context: &StoreContext,
) -> (RiskReportResponse, HumanCompletePlanQualityResponse) {
    let generated_at = UbuTimestamp::now_utc().to_string();
    let steps = input
        .selected_candidate
        .map(|candidate| candidate.steps.as_slice())
        .unwrap_or_default();
    let recovery_present = steps.iter().any(|step| {
        context
            .tasks
            .get(&step.task_id)
            .is_some_and(is_recovery_task)
    });
    let margin_signal = input
        .legitimization
        .and_then(|report| report.affect_margin)
        .or_else(|| {
            input
                .selected_candidate
                .and_then(|candidate| candidate.feasibility_summary.minimum_affect_score)
                .map(|score| score - minimum_affect_threshold(input.request))
        });
    let margin = margin_signal.unwrap_or(0.0);
    let violated_dimensions = input
        .legitimization
        .map(|report| report.violated_dimensions.clone())
        .or_else(|| {
            input.selected_candidate.map(|candidate| {
                candidate
                    .feasibility_summary
                    .violated_affect_dimensions
                    .clone()
            })
        })
        .unwrap_or_default();
    let projection = post_plan_affect_projection(margin, recovery_present);
    let checkpoint_coverage = checkpoint_coverage(steps, &context.tasks);
    let failure_pattern = failure_pattern(&context.recent_logs);

    let plan_quality = HumanCompletePlanQualityResponse {
        generated_at: generated_at.clone(),
        plan_ref: input.plan_ref.to_owned(),
        feedback_latency: feedback_latency(steps, &context.tasks),
        checkpoint_coverage,
        affect_margin: margin,
        violated_dimensions: violated_dimensions.clone(),
        failure_pattern,
        stretch_pressure: projection.stretch_pressure,
        post_plan_state_delta: projection.post_plan_state_delta,
        revision_suggestions: revision_suggestions(
            checkpoint_coverage,
            failure_pattern,
            projection,
            context.active_worker_count,
        ),
    };

    let mut findings = Vec::new();
    deadline_findings(steps, &context.tasks, &mut findings);
    dependency_findings(steps, input.selected_candidate, &mut findings);
    if context.active_worker_count > 1 {
        findings.push(RiskFinding {
            category: RiskCategory::WorkerBottleneck,
            severity: if context.active_worker_count > 3 {
                RiskLevel::High
            } else {
                RiskLevel::Medium
            },
            blocking: false,
            detail: format!(
                "{} worker or automation submissions are simultaneously active",
                context.active_worker_count
            ),
            subject_ref: None,
        });
    }
    if input.legitimization.is_some_and(|report| {
        !report.stale_dimensions.is_empty()
            || report
                .stale_affect_warning
                .as_deref()
                .is_some_and(|warning| warning.contains("stale affect"))
    }) || input
        .request
        .affect_warning
        .as_deref()
        .is_some_and(|warning| warning.contains("stale affect"))
    {
        findings.push(RiskFinding {
            category: RiskCategory::StaleAffect,
            severity: RiskLevel::Medium,
            blocking: false,
            detail: "the affect Snapshot is older than the configured freshness horizon".to_owned(),
            subject_ref: None,
        });
    }
    if let Some(margin) = margin_signal {
        if margin <= NEAR_AFFECT_MARGIN {
            findings.push(RiskFinding {
                category: RiskCategory::AffectMargin,
                severity: if margin < 0.0 {
                    RiskLevel::High
                } else {
                    RiskLevel::Medium
                },
                blocking: false,
                detail: format!("projected affect margin is {margin:.3}, near or below its limit"),
                subject_ref: violated_dimensions.first().cloned(),
            });
        }
        if projection.stretch_pressure == StretchPressure::DestructivePressure {
            findings.push(RiskFinding {
                category: RiskCategory::DestructivePressure,
                severity: RiskLevel::High,
                blocking: true,
                detail: "the projected margin crosses an affect limit without a recovery step"
                    .to_owned(),
                subject_ref: violated_dimensions.first().cloned(),
            });
        }
        if matches!(
            projection.post_plan_state_delta,
            PostPlanStateDelta::Depleted | PostPlanStateDelta::AtRisk
        ) {
            findings.push(RiskFinding {
                category: RiskCategory::PostPlanDepletion,
                severity: if projection.post_plan_state_delta == PostPlanStateDelta::AtRisk {
                    RiskLevel::High
                } else {
                    RiskLevel::Medium
                },
                blocking: false,
                detail: "the margin-based projection ends in a depleted or at-risk band".to_owned(),
                subject_ref: None,
            });
        }
    }
    skeleton_findings(
        input.diagnostics,
        steps.first(),
        input.request,
        &mut findings,
    );
    // No compact-calendar coverage estimate exists in this response, so low_coverage is
    // deliberately omitted instead of being inferred from rollout probability.

    let level = findings
        .iter()
        .map(|finding| finding.severity)
        .max()
        .unwrap_or(RiskLevel::Low);
    (
        RiskReportResponse {
            generated_at,
            level,
            findings,
        },
        plan_quality,
    )
}

#[derive(Debug, Clone, Copy)]
struct AffectProjection {
    stretch_pressure: StretchPressure,
    post_plan_state_delta: PostPlanStateDelta,
}

fn post_plan_affect_projection(margin: f64, recovery_present: bool) -> AffectProjection {
    // TODO(affect-trajectory): Replace the margin-based projection with a per-task
    // `affect_delta` input (a future Task-model field) summed under a clamped,
    // non-linear forward trajectory reusing the §6 sigmoid family, with inter-task
    // recovery. Once it becomes a real trajectory it should move into the kernel
    // (which owns the §6 affect model), with the orchestrator consuming a
    // kernel-emitted trajectory rather than recomputing it. This seam exists so
    // that swap is localized.
    let stretch_pressure = if margin < 0.0 && !recovery_present {
        StretchPressure::DestructivePressure
    } else if margin < 0.25 {
        StretchPressure::SustainableStretch
    } else {
        StretchPressure::Comfort
    };
    let post_plan_state_delta = if margin < 0.0 {
        PostPlanStateDelta::AtRisk
    } else if margin < 0.15 {
        PostPlanStateDelta::Depleted
    } else if margin < 0.35 || !recovery_present {
        PostPlanStateDelta::Neutral
    } else {
        PostPlanStateDelta::Better
    };
    AffectProjection {
        stretch_pressure,
        post_plan_state_delta,
    }
}

fn deadline_findings(
    steps: &[ScheduledTaskBody],
    tasks: &HashMap<String, Value>,
    findings: &mut Vec<RiskFinding>,
) {
    for step in steps {
        let Some(due_at) = tasks
            .get(&step.task_id)
            .and_then(|task| task.get("due_at"))
            .and_then(schedule_coordinate)
        else {
            continue;
        };
        if step.end > due_at {
            findings.push(RiskFinding {
                category: RiskCategory::DeadlineRisk,
                severity: RiskLevel::High,
                blocking: true,
                detail: format!(
                    "scheduled end {} exceeds the hard due coordinate {}",
                    step.end, due_at
                ),
                subject_ref: Some(step.task_id.clone()),
            });
        } else if due_at.saturating_sub(step.end) <= LITTLE_SLACK_SECONDS {
            findings.push(RiskFinding {
                category: RiskCategory::DeadlineRisk,
                severity: RiskLevel::Medium,
                blocking: false,
                detail: format!(
                    "scheduled end leaves {} seconds or planner units before the due coordinate",
                    due_at.saturating_sub(step.end)
                ),
                subject_ref: Some(step.task_id.clone()),
            });
        }
    }
}

fn dependency_findings(
    steps: &[ScheduledTaskBody],
    candidate: Option<&PlanCandidateBody>,
    findings: &mut Vec<RiskFinding>,
) {
    let ends = steps
        .iter()
        .map(|step| (step.task_id.as_str(), step.end))
        .collect::<HashMap<_, _>>();
    let fragile = steps.iter().find_map(|step| {
        step.depends_on.iter().find_map(|dependency| {
            ends.get(dependency.as_str()).and_then(|dependency_end| {
                (step.start.saturating_sub(*dependency_end) <= LITTLE_SLACK_SECONDS)
                    .then(|| step.task_id.clone())
            })
        })
    });
    let kernel_flag = candidate.is_some_and(|candidate| {
        candidate
            .semi_legitimization_summary
            .dependency_fragility_ok
            == Some(false)
    });
    if fragile.is_some() || kernel_flag {
        findings.push(RiskFinding {
            category: RiskCategory::DependencyFragility,
            severity: RiskLevel::Medium,
            blocking: false,
            detail: "a dependency chain has no more than the Phase 1 slack allowance".to_owned(),
            subject_ref: fragile,
        });
    }
}

fn skeleton_findings(
    diagnostics: &[DiagnosticBody],
    first_step: Option<&ScheduledTaskBody>,
    request: &PlanningRequestBody,
    findings: &mut Vec<RiskFinding>,
) {
    let recommendation_ref = first_step.map(|step| step.task_id.as_str()).or_else(|| {
        request
            .task_graph
            .as_ref()
            .and_then(|graph| graph.topological_order.first().map(String::as_str))
    });
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.eq_ignore_ascii_case("skeletonfailure"))
    {
        let subject_ref = skeleton_subject(&diagnostic.message);
        let blocking = subject_ref.as_deref().is_some_and(|subject| {
            recommendation_ref.is_some_and(|recommendation| recommendation == subject)
        });
        findings.push(RiskFinding {
            category: RiskCategory::SkeletonFailure,
            severity: if blocking {
                RiskLevel::High
            } else {
                RiskLevel::Medium
            },
            blocking,
            detail: diagnostic.message.clone(),
            subject_ref,
        });
    }
}

fn skeleton_subject(message: &str) -> Option<String> {
    let rest = message.split(" for task ").nth(1)?;
    let subject = rest.split(':').next()?.trim();
    (!subject.is_empty()).then(|| subject.to_owned())
}

fn feedback_latency(steps: &[ScheduledTaskBody], tasks: &HashMap<String, Value>) -> u64 {
    let Some(plan_start) = steps.iter().map(|step| step.start).min() else {
        return 0;
    };
    steps
        .iter()
        .filter(|step| tasks.get(&step.task_id).is_some_and(is_observation_task))
        .map(|step| step.end.saturating_sub(plan_start))
        .min()
        .or_else(|| {
            steps
                .iter()
                .map(|step| step.end.saturating_sub(plan_start))
                .max()
        })
        .unwrap_or(0)
}

fn checkpoint_coverage(
    steps: &[ScheduledTaskBody],
    tasks: &HashMap<String, Value>,
) -> CheckpointCoverage {
    let observation_count = steps
        .iter()
        .filter(|step| tasks.get(&step.task_id).is_some_and(is_observation_task))
        .count();
    match observation_count {
        0 => CheckpointCoverage::Absent,
        1 => CheckpointCoverage::Sparse,
        _ => CheckpointCoverage::Adequate,
    }
}

fn is_observation_task(task: &Value) -> bool {
    [
        "checkpoint",
        "is_checkpoint",
        "observable_evidence",
        "observation_point",
    ]
    .iter()
    .any(|key| task.get(key).is_some_and(truthy))
        || string_array_contains(task.get("tags"), &["checkpoint", "evidence", "review"])
}

fn is_recovery_task(task: &Value) -> bool {
    ["recovery", "is_recovery", "recovery_step"]
        .iter()
        .any(|key| task.get(key).is_some_and(truthy))
        || string_array_contains(task.get("tags"), &["recovery", "break", "buffer"])
}

fn truthy(value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| value.as_str().is_some_and(|value| !value.trim().is_empty()))
}

fn string_array_contains(value: Option<&Value>, needles: &[&str]) -> bool {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|value| {
            needles
                .iter()
                .any(|needle| value.eq_ignore_ascii_case(needle))
        })
}

fn failure_pattern(logs: &[LogContext]) -> FailurePattern {
    for log in logs {
        let text = format!("{} {}", log.event_type, log.payload).to_ascii_lowercase();
        if text.contains("changed_objective") || text.contains("objective_changed") {
            return FailurePattern::ChangedObjective;
        }
        if text.contains("missing_depend") || text.contains("dependency") {
            return FailurePattern::MissingDependencies;
        }
        if text.contains("wrong_estimate") || text.contains("estimate") || text.contains("overrun")
        {
            return FailurePattern::WrongEstimates;
        }
        if text.contains("stale_affect") || text.contains("stale affect") {
            return FailurePattern::StaleAffect;
        }
        if text.contains("interrupt") || text.contains("snooz") {
            return FailurePattern::Interruption;
        }
        if text.contains("overload") || text.contains("capacity") {
            return FailurePattern::Overload;
        }
    }
    FailurePattern::None
}

fn revision_suggestions(
    checkpoint_coverage: CheckpointCoverage,
    failure_pattern: FailurePattern,
    projection: AffectProjection,
    active_worker_count: usize,
) -> Vec<String> {
    let mut suggestions = Vec::new();
    if checkpoint_coverage != CheckpointCoverage::Adequate {
        suggestions
            .push("Add observable checkpoints to shorten the plan's feedback loop.".to_owned());
    }
    match failure_pattern {
        FailurePattern::WrongEstimates => suggestions
            .push("Split uncertain Tasks and revise their duration estimates.".to_owned()),
        FailurePattern::MissingDependencies => {
            suggestions.push("Repair missing dependency edges before the next schedule.".to_owned())
        }
        FailurePattern::StaleAffect => suggestions
            .push("Refresh the affect Snapshot before recalculating the plan.".to_owned()),
        FailurePattern::Interruption => {
            suggestions.push("Add interruption buffers and smaller resumable Tasks.".to_owned())
        }
        FailurePattern::Overload => {
            suggestions.push("Reduce concurrent work and delegate eligible Tasks.".to_owned())
        }
        FailurePattern::ChangedObjective => {
            suggestions.push("Rebuild the Task graph around the changed objective.".to_owned())
        }
        FailurePattern::None => {}
    }
    if projection.stretch_pressure != StretchPressure::Comfort {
        suggestions.push("Add recovery time or reduce the plan's stretch load.".to_owned());
    }
    if active_worker_count > 1 {
        suggestions.push("Serialize or delegate contending automation work.".to_owned());
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions
}

fn minimum_affect_threshold(request: &PlanningRequestBody) -> f64 {
    request
        .affect_profile
        .as_ref()
        .and_then(|profile| {
            profile
                .dimensions
                .values()
                .map(|dimension| dimension.threshold)
                .reduce(f64::min)
        })
        .unwrap_or(0.5)
}

fn schedule_coordinate(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let timestamp = UbuTimestamp::parse(value.as_str()?).ok()?;
        let seconds = timestamp.inner().unix_timestamp();
        (seconds >= 0).then_some(seconds as u64 / 60)
    })
}

async fn load_store_context(pool: &sqlx::SqlitePool) -> Result<StoreContext> {
    let task_rows = sqlx::query("SELECT id, payload_json FROM objects WHERE object_type = ?")
        .bind(ObjectType::Task.as_str())
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut tasks = HashMap::new();
    for row in task_rows {
        let id: String = row
            .try_get("id")
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let payload = serde_json::from_str(&payload_json).map_err(|error| {
            AppError::Internal(format!(
                "failed to deserialize Task for planning analysis: {error}"
            ))
        })?;
        tasks.insert(id, payload);
    }

    let log_rows =
        sqlx::query("SELECT event_type, payload_json FROM logs ORDER BY created_at DESC LIMIT ?")
            .bind(RECENT_LOG_LIMIT)
            .fetch_all(pool)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
    let recent_logs = log_rows
        .into_iter()
        .map(|row| {
            let event_type: String = row
                .try_get("event_type")
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let payload_json: String = row
                .try_get("payload_json")
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let payload = serde_json::from_str(&payload_json).map_err(|error| {
                AppError::Internal(format!(
                    "failed to deserialize Log for planning analysis: {error}"
                ))
            })?;
            Ok(LogContext {
                event_type,
                payload,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let active_worker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM worker_submissions WHERE lower(status) IN ('pending', 'queued', 'submitted', 'running', 'in_progress')",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;

    Ok(StoreContext {
        tasks,
        recent_logs,
        active_worker_count: active_worker_count.max(0) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::planning::{
        CandidateRoleBody, FeasibilitySummaryBody, ProbabilityQualityBody, ScoreSummaryBody,
        SemiLegitimizationResultBody, SemiLegitimizationSummaryBody,
    };

    fn candidate(steps: Vec<ScheduledTaskBody>, margin: f64) -> PlanCandidateBody {
        PlanCandidateBody {
            candidate_id: "candidate".to_owned(),
            rank: 1,
            candidate_role: CandidateRoleBody::HighestUtility,
            steps,
            score_summary: ScoreSummaryBody {
                utility_score: 1.0,
                robustness_score: 1.0,
                affect_margin_score: margin,
                schedule_diversity_score: 0.0,
                total_score: 1.0,
            },
            feasibility_summary: FeasibilitySummaryBody {
                hard_constraints_assumed_satisfied_by_engine: true,
                affect_feasible: margin >= 0.0,
                minimum_affect_score: Some(0.5 + margin),
                violated_affect_dimensions: if margin < 0.0 {
                    vec!["energy".to_owned()]
                } else {
                    Vec::new()
                },
            },
            semi_legitimization_summary: SemiLegitimizationSummaryBody {
                result: SemiLegitimizationResultBody::PassesCheapChecks,
                affect_budget_ok: Some(margin >= 0.0),
                slack_preserved: Some(true),
                dependency_fragility_ok: Some(true),
                user_mode_compatible: Some(true),
                local_repair_viable: Some(true),
                legitimacy_delta_estimate: Some(0.0),
            },
            display_probability: Some(0.8),
            probability_interval_low: Some(0.7),
            probability_interval_high: Some(0.9),
            robustness_score: 1.0,
            probability_quality: ProbabilityQualityBody::Full,
        }
    }

    fn request() -> PlanningRequestBody {
        serde_json::from_value(serde_json::json!({
            "request_id": "request",
            "tasks": []
        }))
        .expect("request")
    }

    #[test]
    fn destructive_pressure_blocks_but_advisory_findings_do_not() {
        let candidate = candidate(Vec::new(), -0.2);
        let context = StoreContext {
            tasks: HashMap::new(),
            recent_logs: Vec::new(),
            active_worker_count: 2,
        };
        let (risk, quality) = derive_reports(
            PlanningAnalysisInput {
                plan_ref: "plan",
                selected_candidate: Some(&candidate),
                legitimization: None,
                diagnostics: &[],
                request: &request(),
            },
            &context,
        );
        assert_eq!(
            quality.stretch_pressure,
            StretchPressure::DestructivePressure
        );
        assert!(risk.findings.iter().any(|finding| {
            finding.category == RiskCategory::DestructivePressure && finding.blocking
        }));
        assert!(risk.findings.iter().any(|finding| {
            finding.category == RiskCategory::WorkerBottleneck && !finding.blocking
        }));
        assert!(!risk
            .findings
            .iter()
            .any(|finding| finding.category == RiskCategory::LowCoverage));
    }

    #[test]
    fn failure_pattern_and_suggestions_are_model_repairs() {
        let candidate = candidate(Vec::new(), 0.4);
        let context = StoreContext {
            tasks: HashMap::new(),
            recent_logs: vec![LogContext {
                event_type: "task_failed".to_owned(),
                payload: serde_json::json!({"cause": "wrong_estimate"}),
            }],
            active_worker_count: 0,
        };
        let (_, quality) = derive_reports(
            PlanningAnalysisInput {
                plan_ref: "plan",
                selected_candidate: Some(&candidate),
                legitimization: None,
                diagnostics: &[],
                request: &request(),
            },
            &context,
        );
        assert_eq!(quality.failure_pattern, FailurePattern::WrongEstimates);
        assert!(quality
            .revision_suggestions
            .iter()
            .all(|suggestion| !suggestion.to_ascii_lowercase().contains("you")));
        assert!(quality
            .revision_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("duration estimates")));
    }

    #[test]
    fn recommendation_path_skeleton_failure_is_blocking() {
        let mut request = request();
        request.task_graph = Some(crate::api::planning::TaskGraphBody {
            topological_order: vec!["task_next".to_owned()],
            edges: Vec::new(),
        });
        let diagnostics = vec![DiagnosticBody {
            code: "SkeletonFailure".to_owned(),
            message: "Could not build deterministic skeleton for task task_next: impossible window"
                .to_owned(),
        }];
        let context = StoreContext {
            tasks: HashMap::new(),
            recent_logs: Vec::new(),
            active_worker_count: 0,
        };

        let (risk, _) = derive_reports(
            PlanningAnalysisInput {
                plan_ref: "request",
                selected_candidate: None,
                legitimization: None,
                diagnostics: &diagnostics,
                request: &request,
            },
            &context,
        );

        assert!(risk.findings.iter().any(|finding| {
            finding.category == RiskCategory::SkeletonFailure
                && finding.blocking
                && finding.subject_ref.as_deref() == Some("task_next")
        }));
    }
}
