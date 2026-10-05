use serde_json::json;
use ubu_core::worker::local_advisory::{
    AdvisoryTransport, LocalAdvisoryResultStatus, LocalAdvisorySubmission,
};
use ubu_core::{
    AdvisoryCandidateId, AuthoritySource, CandidateLifecycleState, MutationEnvelope,
    ResurfaceTrigger, RetentionPolicy, UbuTimestamp, VersionRef,
};
use ubu_store::api::admission::{
    admit_advisory_candidate, transition_advisory_candidate,
    RejectionInput,
};
use ubu_store::api::review::{get_advisory_candidate, CandidateRecord};
use ubu_store::candidates::store_advisory_candidate;
use ubu_store::models::object_record::{NewObjectRecord, ObjectRecord};

use crate::errors::{AppError, Result};
use crate::services::proposal_applier::{apply_clarification, apply_proposal, proposal_target};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AdvisoryRunReport {
    pub submission_id: String,
    pub status: LocalAdvisoryResultStatus,
    pub candidates_stored: usize,
    pub candidates_rejected: usize,
    pub candidates_suppressed: usize,
    pub candidate_ids: Vec<String>,
    pub proposals: Vec<serde_json::Value>,
    pub diagnostics: Vec<serde_json::Value>,
    pub validation_error: Option<String>,
}

/// The controller is the only path from a worker result to the candidate store.
/// The transport receives only the by-value submission; it has no AppState or
/// store handle. Candidate storage uses the candidate's deterministic key.
pub async fn run_advisory<T: AdvisoryTransport + ?Sized>(
    state: &AppState,
    mut submission: LocalAdvisorySubmission,
    transport: &T,
) -> Result<AdvisoryRunReport> {
    submission.validate()?;
    let _guard = state.inner().advisory_run_lock.lock().await;
    let mut policy_diagnostics = Vec::new();
    let mut ask = true;
    if submission.expected_result_schema == super::precondition_review::RESULT_SCHEMA {
        let mut context: super::precondition_review::Context = serde_json::from_value(submission.payload.clone()).map_err(|e|AppError::Internal(e.to_string()))?;
        (context.tasks, policy_diagnostics) = super::review_policy::eligible(state, context.tasks, context.force).await?;
        ask = !context.tasks.is_empty();
        submission.payload = serde_json::to_value(context).expect("context serializes");
    }
    let mut result = if submission.expected_result_schema == super::precondition_review::RESULT_SCHEMA { super::precondition_review::submit_reviews(&submission, transport) } else if ask { transport.submit(&submission).unwrap_or_else(|_| super::advisory_wire::failed(&submission, super::advisory_wire::Failure::Connection)) } else { super::advisory_wire::empty_result(&submission) };
    result.diagnostics.splice(0..0, policy_diagnostics);
    let mut report = AdvisoryRunReport {
        submission_id: submission.submission_id.clone(),
        status: result.status,
        candidates_stored: 0,
        candidates_rejected: 0,
        candidates_suppressed: 0,
        candidate_ids: Vec::new(),
        proposals: result.proposed_candidates.iter().filter(|candidate| candidate.candidate_kind != ubu_core::CandidateKind::Precondition).map(|candidate| json!({"target_refs":candidate.target_refs,"normalized_proposal":candidate.normalized_proposal,"confidence":candidate.confidence})).collect(),
        diagnostics: result.diagnostics.clone(),
        validation_error: None,
    };

    if let Err(error) = result.validate_against(&submission) {
        report.candidates_rejected = result.proposed_candidates.len();
        report.status = LocalAdvisoryResultStatus::MalformedResult;
        report.diagnostics.push(json!({"code":"advisory_invalid_result","message":error.to_string()}));
        report.validation_error = Some(error.to_string());
        return Ok(report);
    }

    if submission.expected_result_schema == super::precondition_review::RESULT_SCHEMA {
        super::precondition_review::vet_result(state, &submission, &mut result).await?;
    } else { super::precondition_advisor::vet_result(state, &mut result).await?; }
    report.status = result.status;
    report.diagnostics = result.diagnostics.clone();
    report.proposals = result.proposed_candidates.iter().map(|candidate| json!({"target_refs":candidate.target_refs,"normalized_proposal":candidate.normalized_proposal,"confidence":candidate.confidence})).collect();

    report.candidates_rejected = result
        .proposed_candidates
        .len()
        .saturating_sub(result.admissible_candidates().len());
    for candidate in result.admissible_candidates() {
        if candidate.lifecycle_state != CandidateLifecycleState::Proposed {
            report.candidates_rejected += 1;
            continue;
        }
        let identity = json!({"candidate_kind":candidate.candidate_kind,"normalized_proposal":candidate.normalized_proposal,"target_refs":candidate.target_refs});
        let derived = String::from_utf8(ubu_core::canonical_payload_bytes(&identity)).expect("canonical JSON is UTF-8");
        let key = candidate.suppression_key.as_deref().unwrap_or(&derived);
        if !super::precondition_review::is_review(candidate) && ubu_store::api::review::find_suppression_record(state.inner().store.pool(),key).await?.is_some() {
            report.candidates_suppressed += 1;
            report.diagnostics.push(json!({"code":"advisory_proposal_suppressed","message":"A previously rejected proposal was suppressed; it was not re-enqueued"}));
            continue;
        }
        if !super::precondition_review::is_review(candidate) && (candidate.normalized_proposal["operation"] == "set_category" || candidate.candidate_kind == ubu_core::CandidateKind::Precondition) {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM advisory_candidates WHERE suppression_key=?)")
                .bind(key).fetch_one(state.inner().store.pool()).await.map_err(|e|AppError::Internal(e.to_string()))?;
            if exists {
                report.diagnostics.push(json!({"code":"advisory_proposal_already_queued","message":"This proposal already has a durable candidate; no duplicate was enqueued"}));
                continue;
            }
        }
        let envelope = state.envelope_with_key(
            Default::default(),
            ubu_core::AuthoritySource::AutomationWorker,
            candidate.effective_time.unwrap_or(candidate.proposed_at),
            candidate.idempotency_key.clone(),
        )?;
        match store_advisory_candidate(state.inner().store.pool(), &envelope, candidate.clone())
            .await
        {
            Ok(record) => {
                report.candidates_stored += 1;
                report.candidate_ids.push(record.advisory_candidate_id);
            },
            Err(ubu_store::StoreError::Core(ubu_core::UbuError::IdempotencyKeyConflict {
                ..
            })) => {
                report.candidates_rejected += 1;
                report.diagnostics.push(json!({
                    "code": "idempotency_key_conflict",
                    "message": "candidate storage key conflicted with a different payload"
                }));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(report)
}

async fn reviewed_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
) -> Result<ubu_core::AdvisoryCandidate> {
    let record = get_advisory_candidate(state.inner().store.pool(), id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("advisory candidate {}", id.as_str())))?;
    if u64::try_from(record.version).ok() != Some(observed_version) {
        return Err(ubu_store::StoreError::PreconditionFailed {
            object_id: id.as_str().to_owned(),
            expected: format!("v{observed_version}"),
            actual: format!("v{}", record.version),
        }
        .into());
    }
    Ok(record.candidate()?)
}

// The read and pure application happen before the atomic writer. Carry the exact
// read version into that writer so any intervening target mutation is rejected.
struct PreparedAdmission {
    envelope: MutationEnvelope,
    record: NewObjectRecord,
}

async fn prepare_admission(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
) -> Result<PreparedAdmission> {
    prepare(state, id, observed_version, apply_proposal).await
}

/// Answering is admitting: the same read, the same envelope and the same atomic
/// writer, with the operator's answers applied in place of the proposal's own change.
async fn prepare_answer(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    answers: &std::collections::BTreeMap<String, String>,
) -> Result<PreparedAdmission> {
    prepare(state, id, observed_version, |candidate, target| {
        apply_clarification(candidate, target, answers)
    })
    .await
}

async fn prepare(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    apply: impl FnOnce(&ubu_core::AdvisoryCandidate, &ObjectRecord) -> Result<NewObjectRecord>,
) -> Result<PreparedAdmission> {
    let candidate = reviewed_candidate(state, id, observed_version).await?;
    let target_ref = proposal_target(&candidate)?;
    let target =
        ubu_store::queries::get_current_state(state.inner().store.pool(), target_ref.id.as_str())
            .await?
            .ok_or_else(|| AppError::TargetNotFound {
                id: target_ref.id.to_string(),
            })?;
    let mut universe_observation = None;
    if candidate.candidate_kind == ubu_core::CandidateKind::Precondition && candidate.normalized_proposal["operation"] != "clear_precondition" {
        let current = super::planning_service::read_current_universe_state(state.inner().store.pool()).await?;
        let Some((universe, version)) = current else {
            return Err(AppError::conflict_diagnostic("advisory_precondition_stale", "The proposal no longer has recorded facts; nothing was admitted"));
        };
        let proposed = if candidate.normalized_proposal["operation"] == "replace_precondition" { &candidate.normalized_proposal["proposed_precondition"] } else { super::precondition_advisor::proposal_trees(&candidate.normalized_proposal)?.0 };
        let missing = super::precondition_advisor::validate_tree(proposed, &universe, crate::instance_mode::MVP_INSTANCE_MODE)
            .map_err(|_| AppError::bad_request_diagnostic("advisory_precondition_invalid", "The proposed precondition cannot be evaluated in this instance"))?;
        if !missing.is_empty() {
            return Err(AppError::conflict_diagnostic("advisory_precondition_stale", "A target required by this proposal is no longer recorded; nothing was admitted"));
        }
        universe_observation = Some((universe.id, VersionRef::Version(u64::try_from(version).map_err(|_| AppError::Internal("invalid UniverseState version".into()))?)));
    }
    let mut record = apply(&candidate, &target)?;
    let version = u64::try_from(target.version)
        .map_err(|_| AppError::Internal("target has an invalid store version".into()))?;
    let mut observations = std::collections::BTreeMap::from([(target_ref.id.clone(), VersionRef::Version(version))]);
    observations.extend(universe_observation);
    let envelope = state.envelope_for(
        observations,
        AuthoritySource::User,
        UbuTimestamp::now_utc(),
    )?;
    record.updated_at = envelope.recorded_time.to_string();
    Ok(PreparedAdmission { envelope, record })
}

impl PreparedAdmission {
    async fn commit(
        self,
        state: &AppState,
        id: &AdvisoryCandidateId,
        observed_version: u64,
    ) -> Result<(CandidateRecord, ObjectRecord)> {
        Ok(admit_advisory_candidate(
            state.inner().store.pool(),
            &self.envelope,
            id,
            observed_version,
            self.record,
        )
        .await?)
    }
}

pub async fn admit_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
) -> Result<(CandidateRecord, ObjectRecord)> {
    let _guard = state.inner().advisory_run_lock.lock().await;
    prepare_admission(state, id, observed_version)
        .await?
        .commit(state, id, observed_version)
        .await
}

pub async fn answer_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    answers: &std::collections::BTreeMap<String, String>,
) -> Result<(CandidateRecord, ObjectRecord)> {
    prepare_answer(state, id, observed_version, answers)
        .await?
        .commit(state, id, observed_version)
        .await
}

pub async fn reject_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    reason: String,
    retention_policy: RetentionPolicy,
) -> Result<CandidateRecord> {
    reject_candidate_for(state, id, observed_version, reason, retention_policy, None).await
}

pub async fn reject_candidate_for(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    reason: String,
    retention_policy: RetentionPolicy,
    snooze_days: Option<u64>,
) -> Result<CandidateRecord> {
    let _guard = state.inner().advisory_run_lock.lock().await;
    let candidate = reviewed_candidate(state, id, observed_version).await?;
    let review = super::precondition_review::is_review(&candidate);
    let context = if review { Some(super::review_policy::decision_context(state, &candidate, snooze_days).await?) } else {
        if snooze_days.is_some() { return Err(AppError::bad_request_diagnostic("advisory_invalid_snooze", "Only admission reviews take a snooze span")); }
        None
    };
    let reason = if review && reason.trim().is_empty() { "No reason provided".into() } else { reason };
    let envelope = state.envelope_for(
        Default::default(),
        AuthoritySource::User,
        state.planning_now(),
    )?;
    Ok(ubu_store::api::admission::reject_advisory_candidate_with_context(
        state.inner().store.pool(),
        &envelope,
        id,
        observed_version,
        RejectionInput {
            rejection_reason_or_user_correction: reason,
            retention_policy,
            evidence_hashes_or_source_fingerprints: Vec::new(),
            suppression_key: None,
        },
        context,
    )
    .await?)
}

pub async fn defer_candidate(state: &AppState, id: &AdvisoryCandidateId, observed_version: u64) -> Result<CandidateRecord> {
    defer_candidate_for(state, id, observed_version, None).await
}
pub async fn defer_candidate_for(state: &AppState, id: &AdvisoryCandidateId, observed_version: u64, snooze_days: Option<u64>) -> Result<CandidateRecord> {
    let _guard = state.inner().advisory_run_lock.lock().await;
    let candidate = reviewed_candidate(state, id, observed_version).await?;
    if !super::precondition_review::is_review(&candidate) {
        if snooze_days.is_some() { return Err(AppError::bad_request_diagnostic("advisory_invalid_snooze", "Only admission reviews take a snooze span")); }
        return review_transition(state,id,observed_version,CandidateLifecycleState::Deferred,None).await;
    }
    let context = super::review_policy::decision_context(state, &candidate, snooze_days).await?;
    let envelope = state.envelope_for(Default::default(), AuthoritySource::User, state.planning_now())?;
    Ok(ubu_store::api::admission::transition_advisory_candidate_with_context(state.inner().store.pool(), &envelope, id, observed_version, CandidateLifecycleState::Deferred, None, Some(context)).await?)
}

pub async fn resurface_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    trigger: ResurfaceTrigger,
) -> Result<CandidateRecord> {
    review_transition(
        state,
        id,
        observed_version,
        CandidateLifecycleState::Resurfaced,
        Some(trigger),
    )
    .await
}

async fn review_transition(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
    next: CandidateLifecycleState,
    trigger: Option<ResurfaceTrigger>,
) -> Result<CandidateRecord> {
    let envelope = state.envelope_for(
        Default::default(),
        AuthoritySource::User,
        UbuTimestamp::now_utc(),
    )?;
    Ok(transition_advisory_candidate(
        state.inner().store.pool(),
        &envelope,
        id,
        observed_version,
        next,
        trigger,
    )
    .await?)
}

#[cfg(test)]
use crate as orchestrator;
#[cfg(test)]
#[path = "../../tests/support/advisory_fixture.rs"]
mod review_fixture;

#[cfg(test)]
mod review_tests {
    use super::*;
    use review_fixture::{ingest, proposal, seed_task, state};

    #[tokio::test]
    async fn concurrent_target_change_after_read_fails_without_partial_admission() {
        let state = state().await;
        let original = seed_task(&state, vec![]).await;
        let candidate = proposal();
        ingest(&state, candidate.clone()).await;
        let id = &candidate.advisory_candidate_id;
        let prepared = prepare_admission(&state, id, 1).await.unwrap();
        assert_eq!(
            prepared
                .envelope
                .observed_versions
                .get(&candidate.target_refs[0].id),
            Some(&VersionRef::Version(1))
        );

        let envelope = state
            .envelope_for(
                [(candidate.target_refs[0].id.clone(), VersionRef::Version(1))]
                    .into_iter()
                    .collect(),
                AuthoritySource::User,
                UbuTimestamp::now_utc(),
            )
            .unwrap();
        let mut changed = prepared.record.clone();
        changed.payload = serde_json::from_str(&original.payload_json).unwrap();
        changed.payload["title"] = json!("Changed concurrently");
        let current =
            ubu_store::api::admission::admit_object(state.inner().store.pool(), &envelope, changed)
                .await
                .unwrap();
        let ledger_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();

        let error = prepared.commit(&state, id, 1).await.unwrap_err();
        assert!(
            matches!(error, AppError::Store(ubu_store::StoreError::PreconditionFailed { ref object_id, .. }) if object_id == &original.id)
        );
        assert_eq!(
            ubu_store::queries::get_current_state(state.inner().store.pool(), &original.id)
                .await
                .unwrap(),
            Some(current)
        );
        let candidate = get_advisory_candidate(state.inner().store.pool(), id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (candidate.lifecycle_state.as_str(), candidate.version),
            ("proposed", 1)
        );
        assert!(ubu_store::api::review::list_candidate_decision_events(
            state.inner().store.pool(),
            id
        )
        .await
        .unwrap()
        .is_empty());
        let ledger_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
        assert_eq!(ledger_after, ledger_before);
    }
    #[tokio::test]
    async fn concurrent_target_change_after_read_fails_without_partially_admitting_an_answer() {
        let state = state().await;
        let original = seed_task(&state, vec![]).await;
        let mut candidate = proposal();
        candidate.candidate_kind = ubu_core::CandidateKind::ClarificationQuestion;
        candidate.confidence = None;
        candidate.normalized_proposal = json!({"operation":"answer_questions","round":1,"questions":[
            {"id":"q1","text":"Is the synthetic review target urgent?","kind":"YesNo"}
        ]});
        ingest(&state, candidate.clone()).await;
        let id = &candidate.advisory_candidate_id;
        let answers = [("q1".to_owned(), "y".to_owned())].into_iter().collect();
        let prepared = prepare_answer(&state, id, 1, &answers).await.unwrap();
        // The answer observes the Task at exactly the version it was read at.
        assert_eq!(
            prepared
                .envelope
                .observed_versions
                .get(&candidate.target_refs[0].id),
            Some(&VersionRef::Version(1))
        );
        assert_eq!(
            prepared.record.payload["description"],
            "Q: Is the synthetic review target urgent?\nA: y\n"
        );

        // Between the read and the write, the Task is edited elsewhere.
        let envelope = state
            .envelope_for(
                [(candidate.target_refs[0].id.clone(), VersionRef::Version(1))]
                    .into_iter()
                    .collect(),
                AuthoritySource::User,
                UbuTimestamp::now_utc(),
            )
            .unwrap();
        let mut changed = prepared.record.clone();
        changed.payload = serde_json::from_str(&original.payload_json).unwrap();
        changed.payload["title"] = json!("Changed concurrently");
        let current =
            ubu_store::api::admission::admit_object(state.inner().store.pool(), &envelope, changed)
                .await
                .unwrap();
        let ledger_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();

        let error = prepared.commit(&state, id, 1).await.unwrap_err();
        assert!(
            matches!(error, AppError::Store(ubu_store::StoreError::PreconditionFailed { ref object_id, .. }) if object_id == &original.id)
        );
        // The Task is as the concurrent edit left it, with no description written.
        let stored =
            ubu_store::queries::get_current_state(state.inner().store.pool(), &original.id)
                .await
                .unwrap();
        assert_eq!(stored, Some(current));
        assert!(!stored.unwrap().payload_json.contains("description"));
        let candidate = get_advisory_candidate(state.inner().store.pool(), id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (candidate.lifecycle_state.as_str(), candidate.version),
            ("proposed", 1)
        );
        assert!(ubu_store::api::review::list_candidate_decision_events(
            state.inner().store.pool(),
            id
        )
        .await
        .unwrap()
        .is_empty());
        let ledger_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
        assert_eq!(ledger_after, ledger_before);
    }
}
