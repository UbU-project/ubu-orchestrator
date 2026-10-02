use std::collections::BTreeMap;

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ubu_core::core::{UniverseMutation, UniverseState};
use utoipa::ToSchema;

use crate::{
    errors::{AppError, Result},
    instance_mode::MVP_INSTANCE_MODE,
    services::universe_state,
    state::AppState,
};

pub const UNIVERSE_STATE_SCHEMA_VERSION: &str = "ubu.orchestrator.universe_state.v1";

/// One `ubu-core` `UniverseMutation`, field for field.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UniverseMutationBody {
    /// `set_fact`, `clear_fact`, `increment_numeric`, `decrement_numeric`,
    /// `add_membership`, `remove_membership` or `append_event_marker`.
    pub operation: String,
    /// `<collection>.<key>`, the spelling a precondition uses.
    pub target: String,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UniverseStateEditRequest {
    pub schema_version: Option<String>,
    pub mutations: Vec<UniverseMutationBody>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UniverseStateResponse {
    pub schema_version: String,
    pub id: String,
    /// The stored version, or null when the store holds no `UniverseState` and
    /// this is the empty state preconditions are evaluated against.
    pub version: Option<i64>,
    pub captured_at: String,
    pub facts: BTreeMap<String, Value>,
    pub numeric_values: BTreeMap<String, f64>,
    pub set_memberships: BTreeMap<String, Vec<Value>>,
    pub event_markers: BTreeMap<String, Vec<Value>>,
    pub source_summary: String,
    pub confidence_summary: Option<String>,
}

fn response(state: UniverseState, version: Option<i64>) -> UniverseStateResponse {
    UniverseStateResponse {
        schema_version: UNIVERSE_STATE_SCHEMA_VERSION.into(),
        id: state.id.to_string(),
        version,
        captured_at: state.captured_at.to_string(),
        facts: state.facts,
        numeric_values: state.numeric_values,
        set_memberships: state
            .set_memberships
            .into_iter()
            .map(|(key, members)| (key, members.into_iter().map(Value::from).collect()))
            .collect(),
        event_markers: state
            .event_markers
            .into_iter()
            .map(|(key, markers)| (key, markers.into_iter().map(Value::Object).collect()))
            .collect(),
        source_summary: state.source_summary,
        confidence_summary: state.confidence_summary,
    }
}

#[utoipa::path(get,path="/universe-state",responses((status=200,body=UniverseStateResponse)))]
pub async fn read(State(state): State<AppState>) -> Result<Json<UniverseStateResponse>> {
    let (current, version) = universe_state::read(&state).await?;
    Ok(Json(response(current, version)))
}

#[utoipa::path(patch,path="/universe-state",request_body=UniverseStateEditRequest,
    responses((status=200,body=UniverseStateResponse),(status=400,description="Unknown schema version, or a mutation was refused; nothing was written"),(status=409,description="The state changed while this edit was being applied")))]
pub async fn edit(
    State(state): State<AppState>,
    Json(request): Json<UniverseStateEditRequest>,
) -> Result<Json<UniverseStateResponse>> {
    match request.schema_version.as_deref() {
        Some(UNIVERSE_STATE_SCHEMA_VERSION) => {}
        Some(other) => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                format!("unsupported schema_version `{other}`"),
            ))
        }
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
    }
    let mutations: Vec<UniverseMutation> = request
        .mutations
        .into_iter()
        .map(|mutation| UniverseMutation {
            operation: mutation.operation,
            target: mutation.target,
            payload: mutation.payload,
            provenance_kind: None,
        })
        .collect();
    let (next, version) = universe_state::apply(&state, &mutations, MVP_INSTANCE_MODE).await?;
    Ok(Json(response(next, Some(version))))
}
