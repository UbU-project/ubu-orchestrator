//! Deliberate, bounded Task selection; titles and IDs are the only model input.
use super::advisory_wire::{SelectedTask, TAG_RESULT_SCHEMA};
use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use ubu_core::worker::{
    AdvisoryCapability, ComputeBudget, LocalAdvisorySubmission, ProviderConfig, WorkerAuthority,
};
use ubu_core::{AdvisoryCandidateId, AuthoritySource, CandidateKind, ObjectType, UbuId};

pub const DEFAULT_LIMIT: usize = 5;
pub const MAX_LIMIT: usize = 25;
pub async fn select(state: &AppState, limit: usize) -> Result<Vec<SelectedTask>> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(AppError::bad_request_diagnostic(
            "advisory_invalid_limit",
            "limit must be between 1 and 25",
        ));
    }
    // An occurrence is rebuilt from its routine's template at the next materialize,
    // so a category admitted onto one would not survive. It is never offered.
    let rows: Vec<String> = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.category_tag') IS NULL AND json_extract(payload_json,'$.occurrence') IS NULL ORDER BY id LIMIT ?")
        .bind(limit as i64).fetch_all(state.inner().store.pool()).await.map_err(|e|AppError::Internal(e.to_string()))?;
    rows.into_iter()
        .map(|raw| {
            let task: ubu_core::core::Task =
                serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.to_string()))?;
            Ok(SelectedTask {
                id: task.id.to_string(),
                title: task.title,
            })
        })
        .collect()
}

/// How many skipped occurrences are named one by one before the rest are counted.
pub const MAX_SKIPPED_NAMED: usize = 3;

/// The uncategorised routine occurrences selection passed over, each named.
pub async fn skipped_occurrences(
    state: &AppState,
) -> Result<Vec<crate::api::planning::DiagnosticBody>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.category_tag') IS NULL AND json_extract(payload_json,'$.occurrence') IS NOT NULL ORDER BY id")
        .fetch_all(state.inner().store.pool()).await.map_err(|e|AppError::Internal(e.to_string()))?;
    let diagnostic = |message: String| crate::api::planning::DiagnosticBody {
        code: "suggest_tags_occurrence_skipped".into(),
        message,
    };
    let mut skipped = Vec::new();
    for raw in rows.iter().take(MAX_SKIPPED_NAMED) {
        let task: ubu_core::core::Task =
            serde_json::from_str(raw).map_err(|e| AppError::Internal(e.to_string()))?;
        skipped.push(diagnostic(format!(
            "Task `{}` ({}) is an occurrence of a routine and was skipped; a routine's category belongs on its template, which the Routines screen edits",
            task.id, task.title
        )));
    }
    if rows.len() > MAX_SKIPPED_NAMED {
        skipped.push(diagnostic(format!(
            "{} more routine occurrences were skipped for the same reason; a routine's category belongs on its template, which the Routines screen edits",
            rows.len() - MAX_SKIPPED_NAMED
        )));
    }
    Ok(skipped)
}

/// `advisory.timeout_ms` is the budget for the whole run, so the CPU budget tracks it.
pub async fn submission(
    state: &AppState,
    tasks: &[SelectedTask],
    model: &str,
) -> Result<LocalAdvisorySubmission> {
    let (timeout_ms, _) = super::setting_authoring::advisory_timeout_ms(state).await?;
    Ok(LocalAdvisorySubmission {
        submission_id: AdvisoryCandidateId::generate().as_str().into(),
        authority: WorkerAuthority {
            worker_id: UbuId::new(ObjectType::AutomationWorker),
            authority_source: AuthoritySource::AutomationWorker,
            granted: [
                AdvisoryCapability::ProposeCandidate(CandidateKind::Tag),
                AdvisoryCapability::EmitDiagnostics,
            ]
            .into_iter()
            .collect(),
            deadline: None,
        },
        payload: serde_json::to_value(tasks).expect("SelectedTask serializes"),
        expected_result_schema: TAG_RESULT_SCHEMA.into(),
        timeout_ms,
        compute_budget: ComputeBudget {
            max_cpu_ms: timeout_ms,
            max_memory_bytes: 512 * 1024 * 1024,
        },
        result_size_limit_bytes: 256 * 1024,
        partial_results_allowed: false,
        causal_parents: vec![],
        observed_policy_versions: Default::default(),
        input_digests: Default::default(),
        provider_config: ProviderConfig {
            provider_name: "ollama".into(),
            provider_version: "api/generate".into(),
            model_name: model.into(),
            model_version: "unspecified".into(),
            prompt_template_digest: None,
        },
        origin_device_id: state.inner().device_registration.device_id.clone(),
        execution_context: None,
        submitted_at: state.planning_now(),
    })
}
