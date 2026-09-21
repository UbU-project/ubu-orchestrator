use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::errors::{AppError, Result};
use crate::state::AppState;

/// Explicitly identifies the review-only candidate-state surface. Candidate
/// rows are never presented as canonical admitted objects.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AdvisoryCandidateResponse {
    pub state_category: String,
    pub candidate: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AdvisoryQueueResponse {
    pub state_category: String,
    pub candidates: Vec<AdvisoryCandidateResponse>,
}

fn candidate_response(payload_json: String) -> Result<AdvisoryCandidateResponse> {
    let candidate = serde_json::from_str(&payload_json).map_err(|error| {
        AppError::Internal(format!("stored candidate payload is invalid: {error}"))
    })?;
    Ok(AdvisoryCandidateResponse {
        state_category: "candidate_state".to_owned(),
        candidate,
    })
}

#[utoipa::path(
    get,
    path = "/advisory/queue",
    responses((status = 200, body = AdvisoryQueueResponse))
)]
pub async fn queue(State(state): State<AppState>) -> Result<Json<AdvisoryQueueResponse>> {
    let records = ubu_store::api::review::review_queue(state.inner().store.pool()).await?;
    let candidates = records
        .into_iter()
        .map(|record| candidate_response(record.payload_json))
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(AdvisoryQueueResponse {
        state_category: "candidate_state".to_owned(),
        candidates,
    }))
}

#[utoipa::path(
    get,
    path = "/advisory/candidate/{candidate_id}",
    params(("candidate_id" = String, Path, description = "Advisory candidate id")),
    responses((status = 200, body = AdvisoryCandidateResponse), (status = 404))
)]
pub async fn candidate(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
) -> Result<Json<AdvisoryCandidateResponse>> {
    let id = ubu_core::AdvisoryCandidateId::parse(&candidate_id)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let record = ubu_store::api::review::get_advisory_candidate(state.inner().store.pool(), &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("advisory candidate {candidate_id}")))?;
    Ok(Json(candidate_response(record.payload_json)?))
}
