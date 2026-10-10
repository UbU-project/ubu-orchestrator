//! Immutable, user-authored affect Snapshots. Recording never recalculates a Plan.
use serde_json::Value;
use sqlx::Row;
use ubu_core::core::{
    AffectDimension, AffectDimensionObservation, AffectDimensions, AffectSourceKind, Snapshot,
    SnapshotAffect,
};
use ubu_core::{AuthoritySource, ObjectType, UbuId, VersionRef};
use ubu_store::{models::object_record::NewObjectRecord, queries};

use crate::{
    api::affect_observation::{
        AffectObservationSourceKind, AffectObservationValues, AffectObservationWriteResponse,
        RecordedAffectObservation, AFFECT_OBSERVATION_SCHEMA_VERSION,
    },
    errors::{AppError, Result},
    state::AppState,
};

pub async fn record(
    state: &AppState,
    values: AffectObservationValues,
) -> Result<AffectObservationWriteResponse> {
    let id = UbuId::new(ObjectType::Snapshot);
    let now = state.planning_now();
    let snapshot = Snapshot {
        id: id.clone(),
        captured_at: now,
        objects: Vec::new(),
        summary: None,
        affect: Some(SnapshotAffect {
            source_kind: AffectSourceKind::LiveObservation,
            observed_at: now,
            dimensions: AffectDimensions {
                energy: AffectDimensionObservation {
                    dimension: AffectDimension::Energy,
                    value: values.energy,
                },
                stress: AffectDimensionObservation {
                    dimension: AffectDimension::Stress,
                    value: values.stress,
                },
                mood_intensity: AffectDimensionObservation {
                    dimension: AffectDimension::MoodIntensity,
                    value: values.mood_intensity,
                },
            },
        }),
    };
    let envelope = state.envelope_for(
        [(id.clone(), VersionRef::Absent)].into_iter().collect(),
        AuthoritySource::User,
        now,
    )?;
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::Snapshot.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "user-capture".into(),
            payload: serde_json::to_value(snapshot)
                .map_err(|error| AppError::Internal(error.to_string()))?,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await?;
    Ok(AffectObservationWriteResponse {
        schema_version: AFFECT_OBSERVATION_SCHEMA_VERSION.into(),
        snapshot_id: id.to_string(),
        observed_at: now.to_string(),
        source_kind: AffectObservationSourceKind::LiveObservation,
        dimension_count: 3,
    })
}

/// Shared selection for GET and store-built planning. Later local admission
/// breaks equal-timestamp ties without changing the immutable observation.
pub(crate) async fn latest_snapshot(pool: &sqlx::SqlitePool) -> Result<Option<(String, Value)>> {
    let row = sqlx::query(
        "SELECT id, payload_json FROM objects
        WHERE object_type = ? AND status = ? AND json_extract(payload_json, '$.affect') IS NOT NULL
        ORDER BY updated_at DESC, rowid DESC LIMIT 1",
    )
    .bind(ObjectType::Snapshot.as_str())
    .bind("active")
    .fetch_optional(pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: String = row
        .try_get("id")
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let json: String = row
        .try_get("payload_json")
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let payload =
        serde_json::from_str(&json).map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Some((id, payload)))
}

pub async fn read(pool: &sqlx::SqlitePool) -> Result<Option<RecordedAffectObservation>> {
    let Some((snapshot_id, payload)) = latest_snapshot(pool).await? else {
        return Ok(None);
    };
    let affect: SnapshotAffect =
        serde_json::from_value(payload["affect"].clone()).map_err(|error| {
            AppError::Internal(format!("invalid stored affect observation: {error}"))
        })?;
    Ok(Some(RecordedAffectObservation {
        snapshot_id,
        observed_at: affect.observed_at.to_string(),
        source_kind: match affect.source_kind {
            AffectSourceKind::LiveObservation => AffectObservationSourceKind::LiveObservation,
            AffectSourceKind::BootstrapDefaultProfile => {
                AffectObservationSourceKind::BootstrapDefaultProfile
            }
        },
        dimensions: AffectObservationValues {
            energy: affect.dimensions.energy.value,
            stress: affect.dimensions.stress.value,
            mood_intensity: affect.dimensions.mood_intensity.value,
        },
    }))
}
