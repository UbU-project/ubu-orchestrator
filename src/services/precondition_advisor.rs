//! Preconditions are proposals over the current vocabulary, never new facts.
use super::{advisory_wire::SelectedTask, planning_service, suggest_tags};
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    state::AppState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use ubu_core::core::{
    evaluate_universe_precondition, validate_precondition_for_mode, InstanceMode, Task,
    UniversePrecondition, UniverseState,
};
use ubu_core::{
    worker::{AdvisoryCapability, LocalAdvisorySubmission},
    CandidateKind,
};

pub const RESULT_SCHEMA: &str = "ubu.advisory.precondition.v1";
pub const PREDICATES: [&str; 7] = [
    "equals",
    "member_of",
    "absent",
    "at_least",
    "at_most",
    "greater_than",
    "less_than",
];
pub const MAX_TARGET_BYTES: usize = 128;
pub const MAX_NODES: usize = 128;
pub const MAX_DEPTH: usize = 16;
pub const MAX_MISSING_NAMED: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribedTask {
    pub id: String,
    pub title: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub tasks: Vec<DescribedTask>,
    pub targets: Vec<String>,
}

/// Only canonical, bounded identifiers may be copied from model output into a
/// missing-target diagnostic. Neither arbitrary prose nor expected values are echoed.
pub fn valid_target(target: &str) -> bool {
    let Some((collection, key)) = target.split_once('.') else {
        return false;
    };
    target.len() <= MAX_TARGET_BYTES
        && matches!(
            collection,
            "facts" | "numeric_values" | "set_memberships" | "event_markers"
        )
        && key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

pub fn targets(state: &UniverseState) -> BTreeSet<String> {
    [
        ("facts", state.facts.keys().collect::<Vec<_>>()),
        ("numeric_values", state.numeric_values.keys().collect()),
        ("set_memberships", state.set_memberships.keys().collect()),
        ("event_markers", state.event_markers.keys().collect()),
    ]
    .into_iter()
    .flat_map(|(collection, keys)| {
        keys.into_iter()
            .map(move |key| format!("{collection}.{key}"))
    })
    .filter(|target| valid_target(target))
    .collect()
}

pub async fn current(state: &AppState) -> Result<UniverseState> {
    Ok(
        planning_service::read_current_universe_state(state.inner().store.pool())
            .await?
            .map(|(value, _)| value)
            .unwrap_or_else(planning_service::synthesized_universe_state),
    )
}

pub async fn select(state: &AppState, limit: usize) -> Result<(Context, Vec<DiagnosticBody>)> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT payload_json FROM objects WHERE object_type='Task' AND status='active' ORDER BY id",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut tasks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut skipped = 0;
    for raw in rows {
        let task: Task =
            serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.to_string()))?;
        let reason = if task.occurrence.is_some() {
            Some("is a routine occurrence; edit its template instead")
        } else if task
            .description
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
        {
            Some("has no description to reason over")
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped += 1;
            if skipped <= suggest_tags::MAX_LIMIT {
                diagnostics.push(DiagnosticBody {
                    code: "precondition_task_skipped".into(),
                    message: format!("Task `{}` {reason}", task.id),
                });
            }
        } else if tasks.len() < limit {
            tasks.push(DescribedTask {
                id: task.id.to_string(),
                title: task.title,
                description: task.description.unwrap(),
            });
        }
    }
    if skipped > suggest_tags::MAX_LIMIT {
        diagnostics.push(DiagnosticBody {
            code: "precondition_task_skipped".into(),
            message: format!(
                "{} more Tasks were skipped: they are routine occurrences or have no description",
                skipped - suggest_tags::MAX_LIMIT
            ),
        });
    }
    let targets = targets(&current(state).await?)
        .into_iter()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        diagnostics.push(DiagnosticBody { code: "precondition_no_facts".into(), message: "No supported fact targets are recorded. Author facts in UniverseState first; no model was asked and no candidate was enqueued.".into() });
    }
    Ok((Context { tasks, targets }, diagnostics))
}

pub async fn submission(
    state: &AppState,
    context: &Context,
    model: &str,
) -> Result<LocalAdvisorySubmission> {
    let selected = context
        .tasks
        .iter()
        .map(|task| SelectedTask {
            id: task.id.clone(),
            title: task.title.clone(),
        })
        .collect::<Vec<_>>();
    let mut submission = suggest_tags::submission(state, &selected, model).await?;
    submission.authority.granted = [
        AdvisoryCapability::ProposeCandidate(CandidateKind::Precondition),
        AdvisoryCapability::EmitDiagnostics,
    ]
    .into_iter()
    .collect();
    submission.payload = serde_json::to_value(context).expect("context serializes");
    submission.expected_result_schema = RESULT_SCHEMA.into();
    Ok(submission)
}

/// Validate every branch, including ones core's boolean evaluator short-circuits.
/// The round trip also refuses unknown/mixed fields that serde's untagged enum
/// could otherwise silently discard. Bounded traversal precedes deserialization.
pub fn validate_tree(
    raw: &Value,
    state: &UniverseState,
    mode: InstanceMode,
) -> std::result::Result<BTreeSet<String>, &'static str> {
    fn shape(raw: &Value, depth: usize, nodes: &mut usize) -> bool {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return false;
        }
        let Some(object) = raw.as_object() else {
            return false;
        };
        for name in ["all_of", "any_of"] {
            if let Some(parts) = object.get(name) {
                return object.len() == 1
                    && parts.as_array().is_some_and(|parts| {
                        !parts.is_empty() && parts.iter().all(|part| shape(part, depth + 1, nodes))
                    });
            }
        }
        let Some(target) = object.get("target").and_then(Value::as_str) else {
            return false;
        };
        let Some(predicate) = object.get("predicate").and_then(Value::as_str) else {
            return false;
        };
        valid_target(target)
            && PREDICATES.contains(&predicate)
            && object
                .keys()
                .all(|key| matches!(key.as_str(), "target" | "predicate" | "expected"))
            && if predicate == "absent" {
                !object.contains_key("expected")
            } else {
                object.contains_key("expected")
            }
    }
    if !shape(raw, 0, &mut 0) {
        return Err("malformed tree");
    }
    let tree: UniversePrecondition =
        serde_json::from_value(raw.clone()).map_err(|_| "malformed tree")?;
    // JSON null is a legitimate equals expectation, but the core Option field
    // cannot represent it. Refuse it rather than silently dropping it.
    if serde_json::to_value(&tree).map_err(|_| "malformed tree")? != *raw {
        return Err("malformed tree");
    }
    validate_precondition_for_mode(mode, &tree).map_err(|_| "invalid mode")?;
    fn visit(
        tree: &UniversePrecondition,
        state: &UniverseState,
        known: &BTreeSet<String>,
        missing: &mut BTreeSet<String>,
    ) -> std::result::Result<(), &'static str> {
        match tree {
            UniversePrecondition::AllOf { all_of } => {
                for part in all_of {
                    visit(part, state, known, missing)?;
                }
            }
            UniversePrecondition::AnyOf { any_of } => {
                for part in any_of {
                    visit(part, state, known, missing)?;
                }
            }
            UniversePrecondition::Leaf(leaf) => {
                evaluate_universe_precondition(state, tree).map_err(|_| "malformed tree")?;
                if !known.contains(&leaf.target) {
                    missing.insert(leaf.target.clone());
                }
            }
        }
        Ok(())
    }
    let mut missing = BTreeSet::new();
    visit(&tree, state, &targets(state), &mut missing)?;
    evaluate_universe_precondition(state, &tree).map_err(|_| "malformed tree")?;
    Ok(missing)
}

pub fn missing_diagnostic(task_id: &str, missing: &BTreeSet<String>) -> Value {
    let names = missing
        .iter()
        .take(MAX_MISSING_NAMED)
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let rest = missing.len().saturating_sub(MAX_MISSING_NAMED);
    let more = if rest == 0 {
        String::new()
    } else {
        format!(" and {rest} more")
    };
    json!({"code":"precondition_missing_targets", "message":format!("Task `{task_id}` needs recorded targets {names}{more}; author the facts in UniverseState before asking again. No candidate was enqueued for this Task.")})
}

/// Recheck against current state at the controller boundary, including injected
/// transports. Complete validation precedes all enqueues, so malformed output
/// cannot leave a partially stored batch.
pub async fn vet_result(
    state: &AppState,
    result: &mut ubu_core::worker::LocalAdvisoryResult,
) -> Result<()> {
    use ubu_core::{worker::LocalAdvisoryResultStatus, ObjectType};
    if !result
        .proposed_candidates
        .iter()
        .any(|candidate| candidate.candidate_kind == CandidateKind::Precondition)
    {
        return Ok(());
    }
    let universe = current(state).await?;
    let mut accepted = Vec::new();
    for mut candidate in std::mem::take(&mut result.proposed_candidates) {
        if candidate.candidate_kind != CandidateKind::Precondition {
            accepted.push(candidate);
            continue;
        }
        let missing = validate_tree(
            &candidate.normalized_proposal,
            &universe,
            crate::instance_mode::MVP_INSTANCE_MODE,
        );
        let target = match candidate.target_refs.as_slice() {
            [target] if target.object_type == ObjectType::Task => Some(target),
            _ => None,
        };
        let (Ok(missing), Some(target)) = (missing, target) else {
            result.status = LocalAdvisoryResultStatus::MalformedResult;
            result.diagnostics = vec![
                json!({"code":"advisory_malformed_result","message":"The model response was not an evaluable precondition for this instance; no candidates were enqueued"}),
            ];
            return Ok(());
        };
        let row =
            ubu_store::queries::get_current_state(state.inner().store.pool(), target.id.as_str())
                .await?;
        let task: Option<Task> = row
            .map(|row| serde_json::from_str(&row.payload_json))
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if task.as_ref().is_none_or(|task| {
            task.status != ubu_core::core::TaskStatus::Active || task.occurrence.is_some()
        }) {
            result.diagnostics.push(json!({"code":"precondition_task_skipped","message":format!("Task `{}` is no longer eligible: it is inactive, absent, or a routine occurrence. Nothing was changed.",target.id)}));
            continue;
        }
        if !missing.is_empty() {
            result
                .diagnostics
                .push(missing_diagnostic(target.id.as_str(), &missing));
            continue;
        }
        // The prior tree comes from canonical state, never model output. It is
        // review context, not model input, and admission will compare it again.
        if let Some(existing) = task.and_then(|task| task.preconditions) {
            candidate.normalized_proposal = json!({
                "existing_precondition": existing,
                "proposed_precondition": candidate.normalized_proposal,
            });
            candidate.payload =
                ubu_core::CandidatePayload::Inline(candidate.normalized_proposal.clone());
        }
        let identity = json!({"candidate_kind":candidate.candidate_kind,"normalized_proposal":candidate.normalized_proposal,"target_refs":candidate.target_refs});
        candidate.suppression_key = Some(
            String::from_utf8(ubu_core::canonical_payload_bytes(&identity))
                .expect("canonical JSON is UTF-8"),
        );
        accepted.push(candidate);
    }
    result.proposed_candidates = accepted;
    Ok(())
}
