use crate::{errors::Result, state::AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Debug, Deserialize, ToSchema)]
pub struct QuickUbuImportRequest {
    pub snapshot_path: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub dry_run: bool,
}
fn default_timezone() -> String {
    "America/New_York".into()
}
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct ImportCounts {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct QuickUbuSkipped {
    pub kind: String,
    pub quick_ubu_id: String,
    pub reason: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct QuickUbuObjectRef {
    pub kind: String,
    pub id: String,
    pub quick_ubu_id: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct QuickUbuImportResponse {
    pub schema_version: String,
    pub dry_run: bool,
    pub routines: ImportCounts,
    pub tasks: ImportCounts,
    pub preferences: ImportCounts,
    pub objectives_not_imported: usize,
    pub skipped: Vec<QuickUbuSkipped>,
    pub diverged: Vec<QuickUbuObjectRef>,
    pub stale: Vec<QuickUbuObjectRef>,
}
#[utoipa::path(post, path = "/import/quick-ubu", request_body = QuickUbuImportRequest,
    responses((status = 200, body = QuickUbuImportResponse), (status = 400, description = "Invalid timezone or snapshot")))]
pub async fn import_quick_ubu(
    State(state): State<AppState>,
    Json(request): Json<QuickUbuImportRequest>,
) -> Result<Json<QuickUbuImportResponse>> {
    Ok(Json(
        crate::services::quick_ubu_import::import(state, request).await?,
    ))
}
