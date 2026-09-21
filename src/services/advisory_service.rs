use serde_json::json;
use ubu_core::worker::local_advisory::{
    AdvisoryTransport, LocalAdvisoryResultStatus, LocalAdvisorySubmission,
};
use ubu_core::CandidateLifecycleState;
use ubu_store::candidates::store_advisory_candidate;

use crate::errors::Result;
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
