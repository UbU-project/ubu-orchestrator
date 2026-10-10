//! A user check-in creates an immutable Snapshot; attribution stays server owned.
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::{
    errors::{AppError, Result},
    services::affect_observation,
    state::AppState,
};

pub const AFFECT_OBSERVATION_SCHEMA_VERSION: &str = "ubu.orchestrator.affect_observation.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordAffectObservationRequest {
    #[schema(value_type = String, required = true)]
    pub schema_version: Option<String>,
    #[schema(value_type = f64, required = true, minimum = 0, maximum = 10)]
    pub energy: Option<Value>,
    #[schema(value_type = f64, required = true, minimum = 0, maximum = 10)]
    pub stress: Option<Value>,
    #[schema(value_type = f64, required = true, minimum = 0, maximum = 10)]
    pub mood_intensity: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct AffectObservationValues {
    pub energy: f64,
    pub stress: f64,
    pub mood_intensity: f64,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AffectObservationSourceKind {
    LiveObservation,
    BootstrapDefaultProfile,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AffectObservationWriteResponse {
    pub schema_version: String,
    pub snapshot_id: String,
    pub observed_at: String,
    pub source_kind: AffectObservationSourceKind,
    pub dimension_count: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecordedAffectObservation {
    pub snapshot_id: String,
    pub observed_at: String,
    pub source_kind: AffectObservationSourceKind,
    pub dimensions: AffectObservationValues,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AffectObservationReadResponse {
    pub schema_version: String,
    pub observation: Option<RecordedAffectObservation>,
}

fn dimension(name: &str, value: Option<Value>) -> Result<f64> {
    let value = value.ok_or_else(|| {
        AppError::bad_request_diagnostic("affect_dimension_missing", format!("{name} is required"))
    })?;
    value
        .as_f64()
        .filter(|value| value.is_finite() && (0.0..=10.0).contains(value))
        .ok_or_else(|| {
            AppError::bad_request_diagnostic(
                "affect_value_out_of_range",
                format!("{name} must be a finite number from 0 to 10"),
            )
        })
}

#[utoipa::path(post, path = "/affect/observation", request_body = RecordAffectObservationRequest,
    responses((status = 201, body = AffectObservationWriteResponse),
        (status = 400, description = "Missing dimension, invalid value or schema version")))]
pub async fn record(
    State(state): State<AppState>,
    Json(request): Json<RecordAffectObservationRequest>,
) -> Result<(StatusCode, Json<AffectObservationWriteResponse>)> {
    match request.schema_version.as_deref() {
        Some(AFFECT_OBSERVATION_SCHEMA_VERSION) => {}
        Some(_) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                "unsupported affect observation schema_version",
            ))
        }
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
    }
    let values = AffectObservationValues {
        energy: dimension("energy", request.energy)?,
        stress: dimension("stress", request.stress)?,
        mood_intensity: dimension("mood_intensity", request.mood_intensity)?,
    };
    Ok((
        StatusCode::CREATED,
        Json(affect_observation::record(&state, values).await?),
    ))
}

#[utoipa::path(get, path = "/affect/observation", responses((status = 200, body = AffectObservationReadResponse)))]
pub async fn read(State(state): State<AppState>) -> Result<Json<AffectObservationReadResponse>> {
    Ok(Json(AffectObservationReadResponse {
        schema_version: AFFECT_OBSERVATION_SCHEMA_VERSION.into(),
        observation: affect_observation::read(state.inner().store.pool()).await?,
    }))
}
