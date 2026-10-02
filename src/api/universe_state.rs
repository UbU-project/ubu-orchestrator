use std::collections::BTreeMap;

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ubu_core::core::{FactProvenance, ProvenanceKind, UniverseMutation, UniverseState};
use utoipa::ToSchema;

use crate::{
    errors::{AppError, Result},
    instance_mode::MVP_INSTANCE_MODE,
    services::universe_state,
    state::AppState,
};

pub const UNIVERSE_STATE_SCHEMA_VERSION: &str = "ubu.orchestrator.universe_state.v1";

/// `ubu-core`'s `ProvenanceKind`, spelled out so the document names its four
/// values. The two conversions below match on every variant, so a kind added to
/// `ubu-core` does not compile here until it is added to this too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKindBody {
    /// A person said so.
    Asserted,
    /// An instrument or a reading.
    Measured,
    /// Computed from other facts.
    Derived,
    /// An advisor suggested it and it has not been confirmed.
    Proposed,
}

impl From<ProvenanceKindBody> for ProvenanceKind {
    fn from(kind: ProvenanceKindBody) -> Self {
        match kind {
            ProvenanceKindBody::Asserted => Self::Asserted,
            ProvenanceKindBody::Measured => Self::Measured,
            ProvenanceKindBody::Derived => Self::Derived,
            ProvenanceKindBody::Proposed => Self::Proposed,
        }
    }
}

impl From<ProvenanceKind> for ProvenanceKindBody {
    fn from(kind: ProvenanceKind) -> Self {
        match kind {
            ProvenanceKind::Asserted => Self::Asserted,
            ProvenanceKind::Measured => Self::Measured,
            ProvenanceKind::Derived => Self::Derived,
            ProvenanceKind::Proposed => Self::Proposed,
        }
    }
}

/// How one value was established, and when. Nothing else.
#[derive(Debug, Serialize, ToSchema)]
pub struct FactProvenanceBody {
    pub kind: ProvenanceKindBody,
    pub recorded_at: String,
}

impl From<FactProvenance> for FactProvenanceBody {
    fn from(entry: FactProvenance) -> Self {
        Self {
            kind: entry.kind.into(),
            recorded_at: entry.recorded_at.to_string(),
        }
    }
}

/// One `ubu-core` `UniverseMutation`, field for field.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UniverseMutationBody {
    /// `set_fact`, `clear_fact`, `set_numeric`, `clear_numeric`,
    /// `increment_numeric`, `decrement_numeric`, `add_membership`,
    /// `remove_membership` or `append_event_marker`.
    pub operation: String,
    /// `<collection>.<key>`, the spelling a precondition uses.
    pub target: String,
    #[serde(default)]
    pub payload: Option<Value>,
    /// How the value this mutation writes was established. Absent means
    /// `asserted`. The two clears refuse it.
    #[serde(default)]
    pub provenance_kind: Option<ProvenanceKindBody>,
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
    /// Keyed by full target, `<collection>.<key>`. A target with no entry has no
    /// recorded provenance; an entry is removed with its value.
    pub fact_provenance: BTreeMap<String, FactProvenanceBody>,
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
        fact_provenance: state
            .fact_provenance
            .into_iter()
            .map(|(target, entry)| (target, entry.into()))
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
            provenance_kind: mutation.provenance_kind.map(Into::into),
        })
        .collect();
    let (next, version) = universe_state::apply(&state, &mutations, MVP_INSTANCE_MODE).await?;
    Ok(Json(response(next, Some(version))))
}
