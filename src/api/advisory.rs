use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::errors::{AppError, Result};
use crate::services::advisory_service;
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
    pub deferred_candidates: Vec<AdvisoryCandidateResponse>,
    pub review_intervals: std::collections::BTreeMap<String, crate::services::review_policy::Interval>,
    pub target_titles: std::collections::BTreeMap<String,String>,
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
    let candidates: Vec<_> = records
        .into_iter()
        .map(|record| candidate_response(record.payload_json))
        .collect::<Result<Vec<_>>>()?;
    // Keep the store's active queue unchanged; expose durable deferrals alongside it.
    let deferred: Vec<String> = sqlx::query_scalar("SELECT payload_json FROM advisory_candidates WHERE lifecycle_state='deferred' ORDER BY review_order,created_at,advisory_candidate_id")
        .fetch_all(state.inner().store.pool()).await.map_err(|e|AppError::Internal(e.to_string()))?;
    let deferred_candidates: Vec<_> = deferred.into_iter().map(candidate_response).collect::<Result<_>>()?;
    let mut target_titles = std::collections::BTreeMap::new();
    for entry in candidates.iter().chain(&deferred_candidates) {
        if let Some(targets) = entry.candidate["target_refs"].as_array() {
            for target in targets {
                if let Some(id) = target["id"].as_str() {
                    if let Some(row) = ubu_store::queries::get_current_state(state.inner().store.pool(),id).await? {
                        let payload: Value = serde_json::from_str(&row.payload_json).map_err(|e|AppError::Internal(e.to_string()))?;
                        if let Some(title) = payload["title"].as_str() { target_titles.insert(id.into(),title.into()); }
                    }
                }
            }
        }
    }
    let mut review_intervals = std::collections::BTreeMap::new();
    for entry in candidates.iter().chain(&deferred_candidates) {
        let candidate: ubu_core::AdvisoryCandidate = serde_json::from_value(entry.candidate.clone()).map_err(|e|AppError::Internal(e.to_string()))?;
        if crate::services::precondition_review::is_review(&candidate) {
            review_intervals.insert(candidate.advisory_candidate_id.as_str().to_owned(), crate::services::review_policy::interval(&state, &candidate).await?);
        }
    }
    Ok(Json(AdvisoryQueueResponse {
        review_intervals,
        state_category: "candidate_state".to_owned(),
        candidates,
        deferred_candidates,
        target_titles,
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

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub observed_version: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectRequest {
    pub observed_version: u64,
    pub snooze_days: Option<u64>,
    pub reason: String,
    #[schema(schema_with = retention_policy_schema)]
    pub retention_policy: ubu_core::RetentionPolicy,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeferRequest {
    pub observed_version: u64,
    pub snooze_days: Option<u64>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResurfaceRequest {
    pub observed_version: u64,
    #[schema(schema_with = resurface_trigger_schema)]
    pub trigger: ubu_core::ResurfaceTrigger,
}

/// The operator's answers, by question id. Answering is what admits a clarification proposal.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    pub observed_version: u64,
    pub answers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdvisoryAdmitResponse {
    pub state_category: String,
    pub candidate: Value,
    pub task: Value,
}

fn retention_policy_schema() -> utoipa::openapi::schema::Object {
    utoipa::openapi::schema::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .enum_values(Some(["retain", "purge_payload"]))
        .build()
}

fn resurface_trigger_schema() -> utoipa::openapi::schema::Object {
    utoipa::openapi::schema::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .enum_values(Some([
            "materially_new_evidence",
            "user_request",
            "policy_review_interval",
            "accepted_change_to_target_or_dependencies",
            "clarification_or_external_reference_arrival",
        ]))
        .build()
}

fn parse_id(id: &str) -> Result<ubu_core::AdvisoryCandidateId> {
    ubu_core::AdvisoryCandidateId::parse(id).map_err(|e| AppError::BadRequest(e.to_string()))
}

// Scope review-specific HTTP diagnostics here; services retain typed store errors.
fn review_error(error: AppError) -> AppError {
    use ubu_core::UbuError;
    use ubu_store::StoreError;
    match &error {
        AppError::Store(StoreError::PreconditionFailed { .. }) => {
            AppError::conflict_diagnostic("PreconditionFailed", error.to_string())
        }
        AppError::Store(StoreError::Core(
            UbuError::InvalidCandidateTransition { .. } | UbuError::InvalidResurfaceTrigger { .. },
        )) => AppError::conflict_diagnostic("InvalidCandidateTransition", error.to_string()),
        AppError::Store(StoreError::CandidateMissing { .. }) => {
            AppError::NotFound(error.to_string())
        }
        AppError::Store(StoreError::Core(UbuError::InvalidSuppressionRecord { .. })) => {
            AppError::BadRequest(error.to_string())
        }
        _ => error,
    }
}

#[utoipa::path(post, path = "/advisory/candidate/{candidate_id}/admit",
    params(("candidate_id" = String, Path)), request_body = ReviewRequest,
    responses((status = 200, body = AdvisoryAdmitResponse), (status = 400), (status = 404), (status = 409), (status = 422)))]
pub async fn admit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> Result<Json<AdvisoryAdmitResponse>> {
    let (candidate, task) =
        advisory_service::admit_candidate(&state, &parse_id(&id)?, request.observed_version)
            .await
            .map_err(review_error)?;
    let candidate = candidate_response(candidate.payload_json)?;
    Ok(Json(AdvisoryAdmitResponse {
        state_category: candidate.state_category,
        candidate: candidate.candidate,
        task: serde_json::from_str(&task.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?,
    }))
}

#[utoipa::path(post, path = "/advisory/candidate/{candidate_id}/answer",
    params(("candidate_id" = String, Path)), request_body = AnswerRequest,
    responses((status = 200, body = AdvisoryAdmitResponse), (status = 400), (status = 404), (status = 409), (status = 422)))]
pub async fn answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<AdvisoryAdmitResponse>> {
    let (candidate, task) = advisory_service::answer_candidate(
        &state,
        &parse_id(&id)?,
        request.observed_version,
        &request.answers,
    )
    .await
    .map_err(review_error)?;
    let candidate = candidate_response(candidate.payload_json)?;
    Ok(Json(AdvisoryAdmitResponse {
        state_category: candidate.state_category,
        candidate: candidate.candidate,
        task: serde_json::from_str(&task.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?,
    }))
}

#[utoipa::path(post, path = "/advisory/candidate/{candidate_id}/reject",
    params(("candidate_id" = String, Path)), request_body = RejectRequest,
    responses((status = 200, body = AdvisoryCandidateResponse), (status = 400), (status = 404), (status = 409)))]
pub async fn reject(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RejectRequest>,
) -> Result<Json<AdvisoryCandidateResponse>> {
    let candidate = advisory_service::reject_candidate_for(
        &state,
        &parse_id(&id)?,
        request.observed_version,
        request.reason,
        request.retention_policy,
        request.snooze_days,
    )
    .await
    .map_err(review_error)?;
    Ok(Json(candidate_response(candidate.payload_json)?))
}

#[utoipa::path(post, path = "/advisory/candidate/{candidate_id}/defer",
    params(("candidate_id" = String, Path)), request_body = DeferRequest,
    responses((status = 200, body = AdvisoryCandidateResponse), (status = 400), (status = 404), (status = 409)))]
pub async fn defer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DeferRequest>,
) -> Result<Json<AdvisoryCandidateResponse>> {
    let candidate =
        advisory_service::defer_candidate_for(&state, &parse_id(&id)?, request.observed_version, request.snooze_days)
            .await
            .map_err(review_error)?;
    Ok(Json(candidate_response(candidate.payload_json)?))
}

#[utoipa::path(post, path = "/advisory/candidate/{candidate_id}/resurface",
    params(("candidate_id" = String, Path)), request_body = ResurfaceRequest,
    responses((status = 200, body = AdvisoryCandidateResponse), (status = 400), (status = 404), (status = 409)))]
pub async fn resurface(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ResurfaceRequest>,
) -> Result<Json<AdvisoryCandidateResponse>> {
    let candidate = advisory_service::resurface_candidate(
        &state,
        &parse_id(&id)?,
        request.observed_version,
        request.trigger,
    )
    .await
    .map_err(review_error)?;
    Ok(Json(candidate_response(candidate.payload_json)?))
}
