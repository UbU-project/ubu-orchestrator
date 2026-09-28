//! Structural Task replacement and its one-step inverse.
use crate::{errors::Result, services::decomposition, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub const CONTAINER_SCHEMA_VERSION: &str = "ubu.orchestrator.container.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DecomposeRequest {
    pub schema_version: Option<String>,
    pub expected_version: i64,
    /// Task fields; blocked_by may name proposed siblings with one-based child:N references.
    pub children: Vec<Value>,
    #[serde(default)]
    pub segment_split_points: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    pub schema_version: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecomposeResponse {
    pub schema_version: String,
    pub container_id: String,
    pub origin_task_id: String,
    pub child_task_ids: Vec<String>,
    pub log_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UndoResponse {
    pub schema_version: String,
    pub container_id: String,
    pub restored_task_id: String,
    pub children_mooted: Vec<String>,
    pub children_left_completed: Vec<String>,
    pub log_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContainerListResponse {
    pub schema_version: String,
    pub containers: Vec<Value>,
}

#[utoipa::path(post, path="/container/{container_id}/undo",
    params(("container_id"=String,Path)), request_body=UndoRequest,
    responses((status=200,body=UndoResponse),(status=400,description="Invalid Container"),(status=409,description="Version conflict")))]
pub async fn undo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UndoRequest>,
) -> Result<Json<UndoResponse>> {
    Ok(Json(decomposition::undo(&state, &id, request).await?))
}

#[utoipa::path(get, path="/containers", responses((status=200,body=ContainerListResponse)))]
pub async fn list(State(state): State<AppState>) -> Result<Json<ContainerListResponse>> {
    Ok(Json(decomposition::list(&state).await?))
}
