//! Human-authored Objectives and routines; identity, provenance and both
//! routine version counters are server controlled.
use crate::{
    errors::{AppError, Result},
    services::objective_authoring,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

pub const OBJECTIVE_SCHEMA_VERSION: &str = "ubu.orchestrator.objective.v1";

/// Keeps an explicit `null` distinct from an absent field: `null` clears.
fn supplied<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateObjectiveRequest {
    pub schema_version: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i64>,
    /// `one_time` (default) or `evergreen`. A routine must be `evergreen`.
    pub mode: Option<String>,
    /// Supplied together with `routine_instance_template`; `schedule_version` is set by the server.
    pub recurrence: Option<Value>,
    /// Supplied together with `recurrence`; `template_version` is set by the server.
    pub routine_instance_template: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditObjectiveRequest {
    pub schema_version: Option<String>,
    pub expected_version: i64,
    #[serde(default, deserialize_with = "supplied")]
    pub title: Option<Value>,
    #[serde(default, deserialize_with = "supplied")]
    pub description: Option<Value>,
    #[serde(default, deserialize_with = "supplied")]
    pub priority: Option<Value>,
    /// One of open, active, satisfied, abandoned. Objectives are withdrawn, never deleted.
    #[serde(default, deserialize_with = "supplied")]
    pub status: Option<Value>,
    #[serde(default, deserialize_with = "supplied")]
    pub recurrence: Option<Value>,
    #[serde(default, deserialize_with = "supplied")]
    pub routine_instance_template: Option<Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectiveWriteResponse {
    pub schema_version: String,
    pub objective_id: String,
    pub version: i64,
    /// Set when a routine template changed: materialized occurrences are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectiveSummary {
    pub objective_id: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    pub mode: String,
    pub is_routine: bool,
    pub version: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectiveListResponse {
    pub schema_version: String,
    pub objectives: Vec<ObjectiveSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectiveReadResponse {
    pub schema_version: String,
    pub objective_id: String,
    pub version: i64,
    pub is_routine: bool,
    pub payload: Value,
}

fn validate_schema(version: Option<&str>) -> Result<()> {
    match version {
        Some(OBJECTIVE_SCHEMA_VERSION) => Ok(()),
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

fn fields(pairs: impl IntoIterator<Item = (&'static str, Option<Value>)>) -> Map<String, Value> {
    pairs
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect()
}

#[utoipa::path(post,path="/objective",request_body=CreateObjectiveRequest,
    responses((status=201,body=ObjectiveWriteResponse),(status=400,description="Missing title, incomplete routine fields, non-evergreen routine, or an Objective the core type rejects")))]
pub async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateObjectiveRequest>,
) -> Result<(StatusCode, Json<ObjectiveWriteResponse>)> {
    validate_schema(request.schema_version.as_deref())?;
    let fields = fields([
        ("title", request.title.map(Value::from)),
        ("description", request.description.map(Value::from)),
        ("priority", request.priority.map(Value::from)),
        ("mode", request.mode.map(Value::from)),
        ("recurrence", request.recurrence),
        (
            "routine_instance_template",
            request.routine_instance_template,
        ),
    ]);
    let (objective_id, version) = objective_authoring::create(&state, fields).await?;
    Ok((
        StatusCode::CREATED,
        Json(ObjectiveWriteResponse {
            schema_version: OBJECTIVE_SCHEMA_VERSION.into(),
            objective_id,
            version,
            notice: None,
        }),
    ))
}

#[utoipa::path(patch,path="/objective/{objective_id}",params(("objective_id"=String,Path)),request_body=EditObjectiveRequest,
    responses((status=200,body=ObjectiveWriteResponse),(status=400,description="Invalid edit"),(status=404,description="Unknown Objective"),(status=409,description="Version conflict")))]
pub async fn edit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<EditObjectiveRequest>,
) -> Result<Json<ObjectiveWriteResponse>> {
    validate_schema(request.schema_version.as_deref())?;
    let fields = fields([
        ("title", request.title),
        ("description", request.description),
        ("priority", request.priority),
        ("status", request.status),
        ("recurrence", request.recurrence),
        (
            "routine_instance_template",
            request.routine_instance_template,
        ),
    ]);
    let edited = objective_authoring::edit(&state, &id, request.expected_version, fields).await?;
    Ok(Json(ObjectiveWriteResponse {
        schema_version: OBJECTIVE_SCHEMA_VERSION.into(),
        objective_id: edited.objective_id,
        version: edited.version,
        notice: edited
            .template_changed
            .then(|| objective_authoring::TEMPLATE_NOTICE.into()),
    }))
}

#[utoipa::path(get,path="/objectives",responses((status=200,body=ObjectiveListResponse)))]
pub async fn list(State(state): State<AppState>) -> Result<Json<ObjectiveListResponse>> {
    let objectives = objective_authoring::objectives(&state)
        .await?
        .into_iter()
        .map(|(row, payload)| ObjectiveSummary {
            objective_id: row.id,
            title: payload["title"].as_str().unwrap_or("").into(),
            status: payload["status"].as_str().unwrap_or(&row.status).into(),
            priority: payload["priority"].as_i64(),
            mode: payload["mode"].as_str().unwrap_or("one_time").into(),
            is_routine: objective_authoring::is_routine(&payload),
            version: row.version,
        })
        .collect();
    Ok(Json(ObjectiveListResponse {
        schema_version: OBJECTIVE_SCHEMA_VERSION.into(),
        objectives,
    }))
}

#[utoipa::path(get,path="/objective/{objective_id}",params(("objective_id"=String,Path)),
    responses((status=200,body=ObjectiveReadResponse),(status=404,description="Unknown Objective")))]
pub async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ObjectiveReadResponse>> {
    let (row, payload) = objective_authoring::get(&state, &id).await?;
    Ok(Json(ObjectiveReadResponse {
        schema_version: OBJECTIVE_SCHEMA_VERSION.into(),
        objective_id: row.id,
        version: row.version,
        is_routine: objective_authoring::is_routine(&payload),
        payload,
    }))
}
