use ubu_core::core::Task;
use ubu_core::{AdvisoryCandidate, CandidateKind, ObjectRef, ObjectType};
use ubu_store::models::object_record::{NewObjectRecord, ObjectRecord};

use crate::errors::{AppError, Result};

/// Validate dispatch and target shape before the service attempts a target read.
pub(crate) fn proposal_target(candidate: &AdvisoryCandidate) -> Result<&ObjectRef> {
    let operation = candidate
        .normalized_proposal
        .get("operation")
        .and_then(|v| v.as_str());
    match (operation, candidate.candidate_kind) {
        (Some("add_tag" | "set_category"), CandidateKind::Tag) => {}
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

/// Pure proposal application: no I/O, clocks, envelopes or candidate mutation.
pub fn apply_proposal(
    candidate: &AdvisoryCandidate,
    target: &ObjectRecord,
) -> Result<NewObjectRecord> {
    let reference = proposal_target(candidate)?;
    if reference.id.as_str() != target.id || target.object_type != "Task" {
        return Err(AppError::BadRequest(
            "proposal target does not match the Task record".into(),
        ));
    }
    let category = candidate.normalized_proposal["operation"] == "set_category";
    let tag = candidate
        .normalized_proposal
        .get(if category { "category_tag" } else { "tag" })
        .and_then(|v| v.as_str())
        .filter(|tag| !tag.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("tag proposals require a non-empty string tag or category_tag".into()))?;
    let mut payload: serde_json::Value = serde_json::from_str(&target.payload_json)
        .map_err(|e| AppError::BadRequest(format!("invalid Task payload: {e}")))?;
    let mut task: Task = serde_json::from_value(payload.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid Task payload: {e}")))?;
    if task.id != reference.id {
        return Err(AppError::BadRequest(
            "Task payload id does not match target".into(),
        ));
    }
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
    Ok(NewObjectRecord {
        id: target.id.clone(),
        object_type: target.object_type.clone(),
        version: target.version,
        status: target.status.clone(),
        compartment_label: target.compartment_label.clone(),
        payload,
        created_at: target.created_at.clone(),
        updated_at: target.updated_at.clone(),
    })
}
