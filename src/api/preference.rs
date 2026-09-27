//! Human-authored pairwise Preferences; attribution is server controlled.
use crate::{
    errors::{AppError, Result},
    services::preference_authoring,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use ubu_core::core::{PreferenceOrder, PreferenceSubjects};
use utoipa::ToSchema;

pub const PREFERENCE_SCHEMA_VERSION: &str = "ubu.orchestrator.preference.v1";
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePreferenceRequest {
    pub schema_version: Option<String>,
    pub task_a: Option<String>,
    pub task_b: Option<String>,
    pub objective_a: Option<String>,
    pub objective_b: Option<String>,
    #[schema(value_type=String)]
    pub order: PreferenceOrder,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnablePreferenceRequest {
    pub schema_version: Option<String>,
    pub expected_version: i64,
    pub enabled: bool,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PreferenceWriteResponse {
    pub schema_version: String,
    pub preference_id: String,
    pub version: i64,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PreferenceSummary {
    pub preference_id: String,
    pub version: i64,
    pub task_a: Option<String>,
    pub task_b: Option<String>,
    pub task_a_title: Option<String>,
    pub task_b_title: Option<String>,
    /// Imported Objective pairs remain readable even though authoring rejects them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_b: Option<String>,
    #[schema(value_type=String)]
    pub order: PreferenceOrder,
    pub enabled: bool,
    pub acquired_date: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PreferenceListResponse {
    pub schema_version: String,
    pub preferences: Vec<PreferenceSummary>,
}
fn validate_schema(version: Option<&str>) -> Result<()> {
    match version {
        Some(PREFERENCE_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}
fn written((preference_id, version): (String, i64)) -> PreferenceWriteResponse {
    PreferenceWriteResponse {
        schema_version: PREFERENCE_SCHEMA_VERSION.into(),
        preference_id,
        version,
    }
}
#[utoipa::path(post,path="/preference",request_body=CreatePreferenceRequest,
    responses((status=201,body=PreferenceWriteResponse),(status=400,description="Invalid subjects, duplicate, contradiction or cycle")))]
pub async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreatePreferenceRequest>,
) -> Result<(StatusCode, Json<PreferenceWriteResponse>)> {
    validate_schema(request.schema_version.as_deref())?;
    if request.objective_a.is_some() || request.objective_b.is_some() {
        if request.objective_a.is_some() && request.objective_a == request.objective_b {
            return Err(AppError::bad_request_diagnostic(
                "preference_self_reference",
                "A Preference cannot compare an Objective with itself",
            ));
        }
        preference_authoring::validate_subjects(
            &state,
            request.task_a.as_deref(),
            request.task_b.as_deref(),
            true,
        )
        .await?;
    }
    let (Some(a), Some(b)) = (request.task_a, request.task_b) else {
        return Err(AppError::bad_request_diagnostic(
            "preference_unknown_task",
            "Both task_a and task_b are required",
        ));
    };
    Ok((
        StatusCode::CREATED,
        Json(written(
            preference_authoring::create(&state, &a, &b, request.order).await?,
        )),
    ))
}
#[utoipa::path(patch,path="/preference/{preference_id}",params(("preference_id"=String,Path)),request_body=EnablePreferenceRequest,
    responses((status=200,body=PreferenceWriteResponse),(status=400,description="Invalid or conflicting enabled relation"),(status=404,description="Unknown Preference"),(status=409,description="Version conflict")))]
pub async fn set_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<EnablePreferenceRequest>,
) -> Result<Json<PreferenceWriteResponse>> {
    validate_schema(request.schema_version.as_deref())?;
    Ok(Json(written(
        preference_authoring::set_enabled(&state, &id, request.enabled, request.expected_version)
            .await?,
    )))
}
#[utoipa::path(delete,path="/preference/{preference_id}",params(("preference_id"=String,Path)),
    responses((status=204,description="Preference withdrawn"),(status=404,description="Unknown Preference")))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode> {
    preference_authoring::delete(&state, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/preferences",responses((status=200,body=PreferenceListResponse)))]
pub async fn list(State(state): State<AppState>) -> Result<Json<PreferenceListResponse>> {
    let mut preferences = Vec::new();
    for (row, p) in preference_authoring::preferences(&state).await? {
        let (task_a, task_b, objective_a, objective_b) = match p.subjects {
            PreferenceSubjects::Tasks { a, b } => {
                (Some(a.to_string()), Some(b.to_string()), None, None)
            }
            PreferenceSubjects::Objectives { a, b } => {
                (None, None, Some(a.to_string()), Some(b.to_string()))
            }
        };
        let mut titles = Vec::new();
        for id in [&task_a, &task_b] {
            titles.push(match id {
                Some(id) => preference_authoring::title(&state, id).await?,
                None => None,
            });
        }
        preferences.push(PreferenceSummary {
            preference_id: row.id,
            version: row.version,
            task_a,
            task_b,
            task_a_title: titles.remove(0),
            task_b_title: titles.remove(0),
            objective_a,
            objective_b,
            order: p.order,
            enabled: p.enabled,
            acquired_date: p.acquired_date.to_string(),
        });
    }
    Ok(Json(PreferenceListResponse {
        schema_version: PREFERENCE_SCHEMA_VERSION.into(),
        preferences,
    }))
}
