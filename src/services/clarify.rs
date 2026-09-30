//! The interview: one Task, one question set, one round at a time. The Task's
//! own description is both what is sent and where the answers are kept.
use super::advisory_wire::{ClarifyContext, CLARIFY_RESULT_SCHEMA};
use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use ubu_core::worker::{
    AdvisoryCapability, ComputeBudget, LocalAdvisorySubmission, ProviderConfig, WorkerAuthority,
};
use ubu_core::{AdvisoryCandidateId, AuthoritySource, CandidateKind, ObjectType, UbuId};

/// The accumulated interview lives in the Task's description, so it is bounded there.
pub const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// Why there is no Task to interview. One code, `clarify_no_task`, four
/// remedies: the condition is the same, what the operator should do differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTask {
    /// No active, non-routine Task exists at all.
    NoActiveTask,
    /// Active Tasks exist and every one has a description.
    AllDescribed,
    /// A named Task that is not active, or does not exist.
    NotActive(String),
    /// A named Task that is a routine occurrence.
    Occurrence(String),
}

impl NoTask {
    pub fn message(&self) -> String {
        match self {
            Self::NoActiveTask => "There is no active Task to interview. Capture a Task first.".into(),
            Self::AllDescribed => {
                "Every active Task already has a description. Choose a Task to interview it again.".into()
            }
            Self::NotActive(id) => format!("Task `{id}` is not an active Task."),
            // The sentence SuggestTags uses when it skips an occurrence, for the same reason.
            Self::Occurrence(id) => format!(
                "Task `{id}` is an occurrence of a routine; a routine's description belongs on its template, which the Routines screen edits."
            ),
        }
    }
}

/// The Task to interview, or which kind of nothing there is.
///
/// A routine occurrence is never selected, named or not: it is rebuilt from its
/// routine's template at the next materialize, so answers admitted onto it
/// would be lost. That is the reason SuggestTags excludes occurrences too.
///
/// With no Task named, only a Task with no description is chosen. That is round
/// one. Going deeper on a Task is the operator's deliberate act, so later
/// rounds are reached by naming it.
pub async fn select(
    state: &AppState,
    task_id: Option<&str>,
) -> Result<std::result::Result<ClarifyContext, NoTask>> {
    const ELIGIBLE: &str = "SELECT payload_json FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.occurrence') IS NULL";
    let pool = state.inner().store.pool();
    let raw: Option<String> = match task_id {
        Some(id) => {
            UbuId::parse(id)
                .and_then(|parsed| parsed.require_object_type(ObjectType::Task))
                .map_err(|_| {
                    AppError::bad_request_diagnostic(
                        "clarify_invalid_task_id",
                        format!("`{id}` is not a Task id"),
                    )
                })?;
            // The named row as it is, so that the reason it cannot be interviewed is the true one.
            let row: Option<(String, String)> = sqlx::query_as("SELECT status, payload_json FROM objects WHERE object_type='Task' AND id=?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .map_err(internal)?;
            match row {
                Some((status, payload)) if status == "active" => {
                    let value: serde_json::Value = serde_json::from_str(&payload).map_err(internal)?;
                    if value.get("occurrence").is_some_and(|occurrence| !occurrence.is_null()) {
                        return Ok(Err(NoTask::Occurrence(id.into())));
                    }
                    Some(payload)
                }
                _ => return Ok(Err(NoTask::NotActive(id.into()))),
            }
        }
        None => {
            // Blank means nothing but spaces, tabs and line ends; SQLite's one-argument trim removes spaces only.
            let first: Option<String> = sqlx::query_scalar(&format!("{ELIGIBLE} AND (json_extract(payload_json,'$.description') IS NULL OR trim(json_extract(payload_json,'$.description'),' '||char(9)||char(10)||char(13))='') ORDER BY id LIMIT 1"))
                .fetch_optional(pool)
                .await
                .map_err(internal)?;
            if first.is_none() {
                let any: Option<String> = sqlx::query_scalar(&format!("{ELIGIBLE} LIMIT 1"))
                    .fetch_optional(pool)
                    .await
                    .map_err(internal)?;
                return Ok(Err(if any.is_none() { NoTask::NoActiveTask } else { NoTask::AllDescribed }));
            }
            first
        }
    };
    let Some(raw) = raw else {
        return Ok(Err(NoTask::NoActiveTask));
    };
    let task: ubu_core::core::Task = serde_json::from_str(&raw).map_err(internal)?;
    let id = task.id.to_string();
    let round = rounds_admitted(state, &id).await? + 1;
    Ok(Ok(ClarifyContext {
        id,
        title: task.title,
        category_tag: task.category_tag,
        tags: task.tags,
        description: task.description.filter(|text| !text.trim().is_empty()),
        round,
    }))
}

/// Rounds the operator has answered. The next round is this plus one.
pub async fn rounds_admitted(state: &AppState, task_id: &str) -> Result<u32> {
    let admitted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM advisory_candidates WHERE candidate_kind='clarification_question' AND lifecycle_state='admitted' AND json_extract(payload_json,'$.target_refs[0].id')=?")
        .bind(task_id).fetch_one(state.inner().store.pool()).await.map_err(internal)?;
    u32::try_from(admitted).map_err(internal)
}

/// One open interview per Task: a question set that is waiting, or was put
/// aside, is answered or rejected before another is asked for.
pub async fn interview_open(state: &AppState, task_id: &str) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM advisory_candidates WHERE candidate_kind='clarification_question' AND lifecycle_state IN ('proposed','resurfaced','deferred') AND json_extract(payload_json,'$.target_refs[0].id')=?)")
        .bind(task_id).fetch_one(state.inner().store.pool()).await.map_err(internal)
}

/// As `suggest_tags::submission`, for one Task, and allowed to propose questions only.
pub async fn submission(
    state: &AppState,
    context: &ClarifyContext,
    model: &str,
) -> Result<LocalAdvisorySubmission> {
    let (timeout_ms, _) = super::setting_authoring::advisory_timeout_ms(state).await?;
    Ok(LocalAdvisorySubmission {
        submission_id: AdvisoryCandidateId::generate().as_str().into(),
        authority: WorkerAuthority {
            worker_id: UbuId::new(ObjectType::AutomationWorker),
            authority_source: AuthoritySource::AutomationWorker,
            granted: [
                AdvisoryCapability::ProposeCandidate(CandidateKind::ClarificationQuestion),
                AdvisoryCapability::EmitDiagnostics,
            ]
            .into_iter()
            .collect(),
            deadline: None,
        },
        payload: serde_json::to_value(context).expect("ClarifyContext serializes"),
        expected_result_schema: CLARIFY_RESULT_SCHEMA.into(),
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
