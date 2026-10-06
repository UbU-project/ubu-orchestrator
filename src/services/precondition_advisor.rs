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
pub const SELECTION_SKIP_CODE: &str = "advisory_task_skipped";
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
pub const MAX_PROPOSALS: usize = 3;
pub const MAX_AWAITING_REVIEW: i64 = 10;

/// Deferred proposals were explicitly set aside and do not occupy this backlog.
pub async fn awaiting_review(state: &AppState) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM advisory_candidates WHERE candidate_kind='precondition' AND lifecycle_state IN ('proposed','resurfaced')")
        .fetch_one(state.inner().store.pool())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribedTask {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

/// Supported targets partition themselves by collection; no Settings configure
/// which predicates a vocabulary can support.
#[derive(Debug, PartialEq, Eq)]
pub struct TargetPartitions {
    pub all: Vec<String>,
    pub numbers: Vec<String>,
    pub memberships: Vec<String>,
}

pub fn partition_targets(vocabulary: &[String]) -> TargetPartitions {
    let all: Vec<_> = vocabulary
        .iter()
        .filter(|target| valid_target(target))
        .cloned()
        .collect();
    let collection = |name: &str| {
        all.iter()
            .filter(|target| {
                target
                    .split_once('.')
                    .is_some_and(|(prefix, _)| prefix == name)
            })
            .cloned()
            .collect()
    };
    TargetPartitions {
        numbers: collection("numeric_values"),
        memberships: collection("set_memberships"),
        all,
    }
}

pub const SCHEMA_LEVELS: usize = 3;
pub const SCHEMA_GROUP_ITEMS: usize = 10;

/// A conservative model grammar, not a relaxation of the admission validator.
/// Three levels of ten-way groups permit at most 1 + 10 + 100 = 111 nodes.
pub fn response_schema(context: &Context) -> Option<Value> {
    let vocabulary = partition_targets(&context.targets);
    if context.tasks.is_empty() || vocabulary.all.is_empty() {
        return None;
    }
    let scalar = json!({"type":["string","number","boolean"]});
    let mut leaves = vec![
        json!({"type":"object","additionalProperties":false,"required":["target","predicate"],"properties":{"target":{"type":"string","enum":vocabulary.all},"predicate":{"const":"absent"}}}),
        json!({"type":"object","additionalProperties":false,"required":["target","predicate","expected"],"properties":{"target":{"type":"string","enum":vocabulary.all},"predicate":{"const":"equals"},"expected":scalar}}),
    ];
    if !vocabulary.memberships.is_empty() {
        leaves.push(json!({"type":"object","additionalProperties":false,"required":["target","predicate","expected"],"properties":{"target":{"type":"string","enum":vocabulary.memberships},"predicate":{"const":"member_of"},"expected":scalar}}));
    }
    if !vocabulary.numbers.is_empty() {
        leaves.push(json!({"type":"object","additionalProperties":false,"required":["target","predicate","expected"],"properties":{"target":{"type":"string","enum":vocabulary.numbers},"predicate":{"type":"string","enum":["at_least","at_most","greater_than","less_than"]},"expected":{"type":"number"}}}));
    }
    let mut definitions = serde_json::Map::new();
    definitions.insert("leaf".into(), json!({"oneOf":leaves}));
    for (name, child) in [("group", "leaf"), ("tree", "group")] {
        let reference = format!("#/$defs/{child}");
        let mut branches = vec![json!({"$ref":"#/$defs/leaf"})];
        for operator in ["all_of", "any_of"] {
            branches.push(json!({"type":"object","additionalProperties":false,"required":[operator],"properties":{operator:{"type":"array","minItems":1,"maxItems":SCHEMA_GROUP_ITEMS,"items":{"$ref":reference}}}}));
        }
        definitions.insert(name.into(), json!({"oneOf":branches}));
    }
    let ids: Vec<_> = context.tasks.iter().map(|task| &task.id).collect();
    Some(
        json!({"type":"object","additionalProperties":false,"required":["proposals"],
        "properties":{"proposals":{"type":"array","maxItems":context.tasks.len().min(MAX_PROPOSALS),"items":{
            "type":"object","additionalProperties":false,"required":["id","precondition"],
            "properties":{"id":{"type":"string","enum":ids},"precondition":{"$ref":"#/$defs/tree"}}}}},
        "$defs":definitions}),
    )
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
        } else if task.title.trim().is_empty()
            && task
                .description
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
        {
            Some("has neither a title nor a description to reason over")
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped += 1;
            if skipped <= suggest_tags::MAX_SKIPPED_NAMED {
                diagnostics.push(DiagnosticBody {
                    code: SELECTION_SKIP_CODE.into(),
                    message: format!("Task `{}` {reason}", task.id),
                });
            }
        } else if tasks.len() < limit {
            tasks.push(DescribedTask {
                id: task.id.to_string(),
                title: task.title,
                description: task.description.filter(|text| !text.trim().is_empty()),
            });
        }
    }
    if skipped > suggest_tags::MAX_SKIPPED_NAMED {
        diagnostics.push(DiagnosticBody {
            code: SELECTION_SKIP_CODE.into(),
            message: format!(
                "{} more Tasks were skipped: they are routine occurrences or have neither a title nor a description",
                skipped - suggest_tags::MAX_SKIPPED_NAMED
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

/// Code-authored refusal reasons. Evaluator text is taken only after strict
/// target/predicate validation, so core can describe the required kind without
/// echoing an expected value or arbitrary model prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRefusal {
    BoundExceeded,
    InvalidGroup,
    MissingLeafFields,
    UnknownLeafFields,
    ExpectedRequired,
    ExpectedForbidden,
    TreeDeserialization,
    NullExpectation,
    ModeRefusal,
    EvaluatorRefusal(String),
    InvalidTaskReference,
}

impl std::fmt::Display for TreeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BoundExceeded => "the tree exceeds 128 nodes or depth 16",
            Self::InvalidGroup => {
                "the tree must contain leaves or single-key boolean groups with non-empty arrays"
            }
            Self::MissingLeafFields => "a leaf requires string target and predicate fields",
            Self::UnknownLeafFields => "a leaf has an unrecognised target, predicate or key",
            Self::ExpectedRequired => "this predicate requires an expected value",
            Self::ExpectedForbidden => "absent forbids an expected value",
            Self::TreeDeserialization => "the tree cannot be decoded without losing fields",
            Self::NullExpectation => {
                "a null expected value cannot be represented by this precondition"
            }
            Self::ModeRefusal => "this instance mode does not permit an intrinsic-affect target",
            Self::EvaluatorRefusal(message) => message,
            Self::InvalidTaskReference => "the proposal must reference exactly one Task",
        })
    }
}

pub fn refusal_diagnostic(task_id: Option<&str>, reason: &TreeRefusal) -> Value {
    proposal_refusal_diagnostic("precondition_proposal_refused", task_id, reason)
}

pub(crate) fn proposal_refusal_diagnostic(code: &str, task_id: Option<&str>, reason: &impl std::fmt::Display) -> Value {
    let subject = task_id.map_or_else(|| "A proposal".to_owned(), |id| format!("Task `{id}`"));
    json!({"code":code, "message":format!("{subject}: {reason}. No candidate was enqueued for this Task; the rest of the run stands.")})
}

/// Validate every branch, including ones core's boolean evaluator short-circuits.
/// The round trip also refuses unknown/mixed fields that serde's untagged enum
/// could otherwise silently discard. Bounded traversal precedes deserialization.
pub fn validate_tree(
    raw: &Value,
    state: &UniverseState,
    mode: InstanceMode,
) -> std::result::Result<BTreeSet<String>, TreeRefusal> {
    fn shape(raw: &Value, depth: usize, nodes: &mut usize) -> std::result::Result<(), TreeRefusal> {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return Err(TreeRefusal::BoundExceeded);
        }
        let Some(object) = raw.as_object() else {
            return Err(TreeRefusal::InvalidGroup);
        };
        for name in ["all_of", "any_of"] {
            if let Some(parts) = object.get(name) {
                let parts = parts
                    .as_array()
                    .filter(|parts| object.len() == 1 && !parts.is_empty())
                    .ok_or(TreeRefusal::InvalidGroup)?;
                for part in parts {
                    shape(part, depth + 1, nodes)?;
                }
                return Ok(());
            }
        }
        let Some(target) = object.get("target").and_then(Value::as_str) else {
            return Err(TreeRefusal::MissingLeafFields);
        };
        let Some(predicate) = object.get("predicate").and_then(Value::as_str) else {
            return Err(TreeRefusal::MissingLeafFields);
        };
        if !valid_target(target)
            || !PREDICATES.contains(&predicate)
            || !object
                .keys()
                .all(|key| matches!(key.as_str(), "target" | "predicate" | "expected"))
        {
            return Err(TreeRefusal::UnknownLeafFields);
        }
        if predicate == "absent" && object.contains_key("expected") {
            return Err(TreeRefusal::ExpectedForbidden);
        }
        if predicate != "absent" && !object.contains_key("expected") {
            return Err(TreeRefusal::ExpectedRequired);
        }
        Ok(())
    }
    shape(raw, 0, &mut 0)?;
    let tree: UniversePrecondition =
        serde_json::from_value(raw.clone()).map_err(|_| TreeRefusal::TreeDeserialization)?;
    // JSON null is a legitimate equals expectation, but the core Option field
    // cannot represent it. Refuse it rather than silently dropping it.
    if serde_json::to_value(&tree).map_err(|_| TreeRefusal::TreeDeserialization)? != *raw {
        return Err(TreeRefusal::NullExpectation);
    }
    validate_precondition_for_mode(mode, &tree).map_err(|_| TreeRefusal::ModeRefusal)?;
    fn visit(
        tree: &UniversePrecondition,
        state: &UniverseState,
        known: &BTreeSet<String>,
        missing: &mut BTreeSet<String>,
    ) -> std::result::Result<(), TreeRefusal> {
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
                evaluate_universe_precondition(state, tree).map_err(evaluator_refusal)?;
                if !known.contains(&leaf.target) {
                    missing.insert(leaf.target.clone());
                }
            }
        }
        Ok(())
    }
    let mut missing = BTreeSet::new();
    visit(&tree, state, &targets(state), &mut missing)?;
    evaluate_universe_precondition(state, &tree).map_err(evaluator_refusal)?;
    Ok(missing)
}

fn evaluator_refusal(error: ubu_core::core::UniversePreconditionError) -> TreeRefusal {
    let ubu_core::core::UniversePreconditionError::Malformed(message) = error;
    TreeRefusal::EvaluatorRefusal(message)
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
/// transports. Each proposal is independent: refuse one without discarding
/// surviving proposals or diagnostics already recorded for the run.
pub async fn vet_result(
    state: &AppState,
    result: &mut ubu_core::worker::LocalAdvisoryResult,
) -> Result<()> {
    use ubu_core::ObjectType;
    if !result
        .proposed_candidates
        .iter()
        .any(|candidate| candidate.candidate_kind == CandidateKind::Precondition)
    {
        return Ok(());
    }
    let universe = current(state).await?;
    let mut accepted = Vec::new();
    let mut missing_tasks = 0;
    let mut refused_tasks = 0;
    let mut skipped_tasks = 0;
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
        let reason = if target.is_none() {
            Some(TreeRefusal::InvalidTaskReference)
        } else {
            missing.as_ref().err().cloned()
        };
        if let Some(reason) = reason {
            refused_tasks += 1;
            if refused_tasks <= suggest_tags::MAX_SKIPPED_NAMED {
                result.diagnostics.push(refusal_diagnostic(
                    target.map(|target| target.id.as_str()),
                    &reason,
                ));
            }
            continue;
        }
        let missing = missing.expect("a refused tree was handled above");
        let target = target.expect("a missing Task reference was handled above");
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
            skipped_tasks += 1;
            if skipped_tasks <= suggest_tags::MAX_SKIPPED_NAMED {
                result.diagnostics.push(json!({"code":"precondition_task_skipped","message":format!("Task `{}` is no longer eligible: it is inactive, absent, or a routine occurrence. Nothing was changed.",target.id)}));
            }
            continue;
        }
        if !missing.is_empty() {
            missing_tasks += 1;
            if missing_tasks <= suggest_tags::MAX_SKIPPED_NAMED {
                result
                    .diagnostics
                    .push(missing_diagnostic(target.id.as_str(), &missing));
            }
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
    if refused_tasks > suggest_tags::MAX_SKIPPED_NAMED {
        result.diagnostics.push(json!({"code":"precondition_proposal_refused","message":format!("{} more Tasks had unevaluable proposals; no candidates were enqueued for those Tasks. The rest of the run stands.", refused_tasks - suggest_tags::MAX_SKIPPED_NAMED)}));
    }
    if skipped_tasks > suggest_tags::MAX_SKIPPED_NAMED {
        result.diagnostics.push(json!({"code":"precondition_task_skipped","message":format!("{} more Tasks are no longer eligible: they are inactive, absent, or routine occurrences. Nothing was changed.", skipped_tasks - suggest_tags::MAX_SKIPPED_NAMED)}));
    }
    if missing_tasks > suggest_tags::MAX_SKIPPED_NAMED {
        result.diagnostics.push(json!({"code":"precondition_missing_targets","message":format!("{} more Tasks need recorded targets; no candidates were enqueued for those Tasks.", missing_tasks - suggest_tags::MAX_SKIPPED_NAMED)}));
    }
    result.proposed_candidates = accepted;
    Ok(())
}

/// Decode review context without trusting a replacement wrapper to discard fields.
/// The stored prior tree may refer to a fact that has since been removed; only
/// the proposed tree needs the current vocabulary/evaluation checks.
pub fn proposal_trees(raw: &Value) -> Result<(&Value, Option<UniversePrecondition>)> {
    let invalid = || {
        AppError::bad_request_diagnostic(
            "advisory_precondition_invalid",
            "The precondition review payload is malformed",
        )
    };
    if raw.get("existing_precondition").is_some() || raw.get("proposed_precondition").is_some() {
        let object = raw.as_object().ok_or_else(invalid)?;
        if object.len() != 2 {
            return Err(invalid());
        }
        let old = object.get("existing_precondition").ok_or_else(invalid)?;
        let proposed = object.get("proposed_precondition").ok_or_else(invalid)?;
        let existing: UniversePrecondition =
            serde_json::from_value(old.clone()).map_err(|_| invalid())?;
        if serde_json::to_value(&existing).map_err(|_| invalid())? != *old {
            return Err(invalid());
        }
        Ok((proposed, Some(existing)))
    } else {
        Ok((raw, None))
    }
}
