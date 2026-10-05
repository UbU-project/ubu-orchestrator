use std::collections::BTreeMap;

use ubu_core::core::Task;
use ubu_core::{AdvisoryCandidate, CandidateKind, ObjectRef, ObjectType};
use ubu_store::models::object_record::{NewObjectRecord, ObjectRecord};

use crate::errors::{AppError, Result};
use crate::services::advisory_wire::{Question, QuestionKind};
use crate::services::clarify::MAX_DESCRIPTION_BYTES;

/// Validate dispatch and target shape before the service attempts a target read.
pub(crate) fn proposal_target(candidate: &AdvisoryCandidate) -> Result<&ObjectRef> {
    let operation = candidate
        .normalized_proposal
        .get("operation")
        .and_then(|v| v.as_str());
    match (operation, candidate.candidate_kind) {
        (Some("add_tag" | "set_category"), CandidateKind::Tag)
        | (Some("answer_questions"), CandidateKind::ClarificationQuestion)
        | (None, CandidateKind::Precondition) => {}
        (Some("replace_precondition"), CandidateKind::Precondition) => {}
        (Some("clear_precondition"), CandidateKind::Precondition) => {}
        _ => {
            return Err(AppError::UnsupportedProposal {
                operation: operation.unwrap_or("<missing or non-string>").to_owned(),
                candidate_kind: candidate.candidate_kind,
            })
        }
    }
    match candidate.target_refs.as_slice() {
        [target] if target.object_type == ObjectType::Task => Ok(target),
        _ => Err(AppError::BadRequest(
            "tag proposals require exactly one Task target".into(),
        )),
    }
}

/// The payload and the Task it holds, once the candidate, the record and the
/// payload all agree on which Task this is.
fn target_task(
    candidate: &AdvisoryCandidate,
    target: &ObjectRecord,
) -> Result<(serde_json::Value, Task)> {
    let reference = proposal_target(candidate)?;
    if reference.id.as_str() != target.id || target.object_type != "Task" {
        return Err(AppError::BadRequest(
            "proposal target does not match the Task record".into(),
        ));
    }
    let payload: serde_json::Value = serde_json::from_str(&target.payload_json)
        .map_err(|e| AppError::BadRequest(format!("invalid Task payload: {e}")))?;
    let task: Task = serde_json::from_value(payload.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid Task payload: {e}")))?;
    if task.id != reference.id {
        return Err(AppError::BadRequest(
            "Task payload id does not match target".into(),
        ));
    }
    Ok((payload, task))
}

fn rewritten(target: &ObjectRecord, payload: serde_json::Value) -> NewObjectRecord {
    NewObjectRecord {
        id: target.id.clone(),
        object_type: target.object_type.clone(),
        version: target.version,
        status: target.status.clone(),
        compartment_label: target.compartment_label.clone(),
        payload,
        created_at: target.created_at.clone(),
        updated_at: target.updated_at.clone(),
    }
}

fn refuse(code: &str, message: impl Into<String>) -> AppError {
    AppError::bad_request_diagnostic(code, message)
}

/// The operator's answers, appended to what the Task already says.
///
/// Answers are written in question order, not in the order of their ids, so
/// the narrative reads as the interview was asked. A blank answer is "no
/// comment" and is dropped. A question whose dependency was not answered as it
/// requires never applied, and its answer is dropped with it.
pub fn compose(
    existing: &str,
    questions: &[Question],
    answers: &BTreeMap<String, String>,
) -> Result<String> {
    if let Some(unknown) = answers
        .keys()
        .find(|id| !questions.iter().any(|question| &question.id == *id))
    {
        return Err(refuse(
            "clarify_unknown_question",
            format!("`{unknown}` is not a question in this set"),
        ));
    }
    let mut composed = existing.to_owned();
    // The answers that count, by question id. A dependency is always an earlier
    // question, so one pass in question order decides relevance.
    let mut kept: BTreeMap<&str, String> = BTreeMap::new();
    for question in questions {
        let Some(answer) = answers.get(&question.id).map(|answer| answer.trim()) else {
            continue;
        };
        if answer.is_empty() {
            continue;
        }
        let answer = match question.kind {
            QuestionKind::YesNo => match answer.to_lowercase().as_str() {
                yes_or_no @ ("y" | "n") => yes_or_no.to_owned(),
                _ => {
                    return Err(refuse(
                        "clarify_invalid_answer",
                        format!("Question `{}` is answered y or n", question.id),
                    ))
                }
            },
            QuestionKind::ShortText => answer.to_owned(),
        };
        let relevant = question.depends_on.as_ref().is_none_or(|(dependency, required)| {
            kept.get(dependency.as_str())
                .is_some_and(|given| given.to_lowercase() == required.trim().to_lowercase())
        });
        if !relevant {
            continue;
        }
        if !composed.is_empty() && !composed.ends_with('\n') {
            composed.push('\n');
        }
        composed.push_str(&format!("Q: {}\nA: {answer}\n", question.text));
        kept.insert(&question.id, answer);
    }
    if kept.is_empty() {
        return Err(refuse(
            "clarify_no_answers",
            "No answer was given to a question that applies; nothing was written. Defer the questions to answer them later, or Reject them to dismiss them",
        ));
    }
    if composed.len() > MAX_DESCRIPTION_BYTES {
        return Err(refuse(
            "clarify_description_too_large",
            format!("The Task's description would be {} bytes; the limit is {MAX_DESCRIPTION_BYTES}. Nothing was written", composed.len()),
        ));
    }
    Ok(composed)
}

/// Pure, as `apply_proposal` is: no I/O, clocks, envelopes or candidate mutation.
/// Answering a question set is what admits it, and what is written is the
/// interview itself, to the Task's description, and nothing else.
pub fn apply_clarification(
    candidate: &AdvisoryCandidate,
    target: &ObjectRecord,
    answers: &BTreeMap<String, String>,
) -> Result<NewObjectRecord> {
    if candidate.candidate_kind != CandidateKind::ClarificationQuestion {
        return Err(refuse(
            "advisory_not_a_clarification",
            "Only a clarification proposal is admitted by answering; use Admit for this proposal",
        ));
    }
    let (mut payload, mut task) = target_task(candidate, target)?;
    if task.status != ubu_core::core::TaskStatus::Active {
        return Err(refuse(
            "advisory_target_inactive",
            "The Task is no longer active; the answers were not written",
        ));
    }
    let questions: Vec<Question> =
        serde_json::from_value(candidate.normalized_proposal["questions"].clone())
            .map_err(|e| AppError::BadRequest(format!("invalid question set: {e}")))?;
    let description = compose(task.description.as_deref().unwrap_or(""), &questions, answers)?;
    task.description = Some(description.clone());
    task.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    ubu_core::validation::validate_task_lifecycle(&task)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    // Preserve unrelated payload fields and their existing wire representation.
    payload["description"] = description.into();
    Ok(rewritten(target, payload))
}

/// Pure proposal application: no I/O, clocks, envelopes or candidate mutation.
pub fn apply_proposal(
    candidate: &AdvisoryCandidate,
    target: &ObjectRecord,
) -> Result<NewObjectRecord> {
    if candidate.candidate_kind == CandidateKind::ClarificationQuestion {
        return Err(AppError::bad_request_diagnostic(
            "advisory_answer_required",
            "A clarification proposal is admitted by answering its questions, not by Admit",
        ));
    }
    match (candidate.candidate_kind, candidate.normalized_proposal["operation"].as_str()) {
        (CandidateKind::Precondition, Some("replace_precondition")) => return apply_review(candidate, target, Some(&candidate.normalized_proposal["proposed_precondition"])),
        (CandidateKind::Precondition, Some("clear_precondition")) => return apply_review(candidate, target, None),
        _ => {}
    }
    // The kind identifies set-preconditions. Replacement review context must
    // still match canonical state; never discard a later operator edit.
    if candidate.candidate_kind == CandidateKind::Precondition {
        let (mut payload, mut task) = target_task(candidate, target)?;
        if task.status != ubu_core::core::TaskStatus::Active || task.occurrence.is_some() {
            return Err(refuse("advisory_target_inactive", "Only an active non-occurrence Task can receive a precondition"));
        }
        let (proposed, existing) = super::precondition_advisor::proposal_trees(&candidate.normalized_proposal)?;
        if task.preconditions != existing {
            return Err(AppError::conflict_diagnostic("advisory_precondition_changed", "The Task's precondition changed since this proposal; review a new proposal before replacing it"));
        }
        let precondition = serde_json::from_value(proposed.clone())
            .map_err(|_| refuse("advisory_precondition_invalid", "The proposed precondition is malformed"))?;
        task.preconditions = Some(precondition);
        task.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
        payload["preconditions"] = proposed.clone();
        return Ok(rewritten(target, payload));
    }
    let category = candidate.normalized_proposal["operation"] == "set_category";
    let tag = candidate
        .normalized_proposal
        .get(if category { "category_tag" } else { "tag" })
        .and_then(|v| v.as_str())
        .filter(|tag| !tag.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("tag proposals require a non-empty string tag or category_tag".into()))?;
    let (mut payload, mut task) = target_task(candidate, target)?;
    if !task.tags.iter().any(|existing| existing == tag) {
        task.tags.push(tag.to_owned());
    }
    if category {
        if task.status != ubu_core::core::TaskStatus::Active {
            return Err(AppError::bad_request_diagnostic("advisory_target_inactive", "The Task is no longer active; category proposal was not admitted"));
        }
        if task.category_tag.as_deref().is_some_and(|current| current != tag) {
            return Err(AppError::conflict_diagnostic("advisory_category_changed", "The Task already has another category; review this stale proposal"));
        }
        task.category_tag = Some(tag.to_owned());
        payload["category_tag"] = tag.into();
        task.validate().map_err(|e|AppError::BadRequest(e.to_string()))?;
    }
    ubu_core::validation::validate_task_lifecycle(&task)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    // Preserve unrelated payload fields and their existing wire representation.
    payload["tags"] = serde_json::json!(task.tags);
    Ok(rewritten(target, payload))
}

/// Both explicit review operations compare the reviewed value before writing.
fn apply_review(candidate: &AdvisoryCandidate, target: &ObjectRecord, proposed: Option<&serde_json::Value>) -> Result<NewObjectRecord> {
    if !super::precondition_review::valid_envelope(&candidate.normalized_proposal) {
        return Err(refuse("advisory_precondition_invalid", "The precondition review payload is malformed"));
    }
    let (mut payload, mut task) = target_task(candidate, target)?;
    if task.status != ubu_core::core::TaskStatus::Active || task.occurrence.is_some() {
        return Err(refuse("advisory_target_inactive", "Only an active non-occurrence Task can receive a reviewed precondition"));
    }
    if serde_json::to_value(&task.preconditions).map_err(|e|AppError::Internal(e.to_string()))? != candidate.normalized_proposal["existing_precondition"] {
        return Err(AppError::conflict_diagnostic("advisory_precondition_changed", "The Task's precondition changed since this review; ask for a fresh review"));
    }
    task.preconditions = proposed.map(|p|serde_json::from_value(p.clone())).transpose().map_err(|_|refuse("advisory_precondition_invalid", "The proposed precondition is malformed"))?;
    task.validate().map_err(|e|AppError::BadRequest(e.to_string()))?;
    if let Some(tree) = proposed { payload["preconditions"] = tree.clone(); }
    else { payload.as_object_mut().expect("Task object").remove("preconditions"); }
    Ok(rewritten(target, payload))
}
