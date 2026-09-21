use serde_json::json;
use ubu_core::worker::local_advisory::{
    AdvisoryTransport, LocalAdvisoryResultStatus, LocalAdvisorySubmission,
};
use ubu_core::{
    AdvisoryCandidateId, AuthoritySource, CandidateLifecycleState, MutationEnvelope,
    ResurfaceTrigger, RetentionPolicy, UbuTimestamp, VersionRef,
};
use ubu_store::api::admission::{
    admit_advisory_candidate, reject_advisory_candidate, transition_advisory_candidate,
    RejectionInput,
};
use ubu_store::api::review::{get_advisory_candidate, CandidateRecord};
use ubu_store::candidates::store_advisory_candidate;
use ubu_store::models::object_record::{NewObjectRecord, ObjectRecord};

use crate::errors::{AppError, Result};
use crate::services::proposal_applier::{apply_proposal, proposal_target};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AdvisoryRunReport {
    pub submission_id: String,
    pub status: LocalAdvisoryResultStatus,
    pub candidates_stored: usize,
    pub candidates_rejected: usize,
    pub diagnostics: Vec<serde_json::Value>,
    pub validation_error: Option<String>,
}

/// The controller is the only path from a worker result to the candidate store.
/// The transport receives only the by-value submission; it has no AppState or
/// store handle. Candidate storage uses the candidate's deterministic key.
pub async fn run_advisory<T: AdvisoryTransport>(
    state: &AppState,
    submission: LocalAdvisorySubmission,
    transport: &T,
) -> Result<AdvisoryRunReport> {
    submission.validate()?;
    let result = transport.submit(&submission)?;
    let mut report = AdvisoryRunReport {
        submission_id: submission.submission_id.clone(),
        status: result.status,
        candidates_stored: 0,
        candidates_rejected: 0,
        diagnostics: result.diagnostics.clone(),
        validation_error: None,
    };

    if let Err(error) = result.validate_against(&submission) {
        report.candidates_rejected = result.proposed_candidates.len();
        report.validation_error = Some(error.to_string());
        return Ok(report);
    }

    report.candidates_rejected = result
        .proposed_candidates
        .len()
        .saturating_sub(result.admissible_candidates().len());
    for candidate in result.admissible_candidates() {
        if candidate.lifecycle_state != CandidateLifecycleState::Proposed {
            report.candidates_rejected += 1;
            continue;
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
            Ok(_) => report.candidates_stored += 1,
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
    let candidate = reviewed_candidate(state, id, observed_version).await?;
    let target_ref = proposal_target(&candidate)?;
    let target =
        ubu_store::queries::get_current_state(state.inner().store.pool(), target_ref.id.as_str())
            .await?
            .ok_or_else(|| AppError::TargetNotFound {
                id: target_ref.id.to_string(),
            })?;
    let mut record = apply_proposal(&candidate, &target)?;
    let version = u64::try_from(target.version)
        .map_err(|_| AppError::Internal("target has an invalid store version".into()))?;
    let envelope = state.envelope_for(
        [(target_ref.id.clone(), VersionRef::Version(version))]
            .into_iter()
            .collect(),
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
    prepare_admission(state, id, observed_version)
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
    let envelope = state.envelope_for(
        Default::default(),
        AuthoritySource::User,
        UbuTimestamp::now_utc(),
    )?;
    Ok(reject_advisory_candidate(
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
    )
    .await?)
}

pub async fn defer_candidate(
    state: &AppState,
    id: &AdvisoryCandidateId,
    observed_version: u64,
) -> Result<CandidateRecord> {
    review_transition(
        state,
        id,
        observed_version,
        CandidateLifecycleState::Deferred,
        None,
    )
    .await
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
