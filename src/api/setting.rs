use crate::{
    errors::{AppError, Result},
    services::setting_authoring,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub const SETTING_SCHEMA_VERSION: &str = "ubu.orchestrator.setting.v1";

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingWriteRequest {
    pub schema_version: Option<String>,
    pub value: Value,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingWriteResponse {
    pub schema_version: String,
    pub setting_id: String,
    pub version: i64,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingSummary {
    pub id: String,
    pub name: String,
    pub value: Value,
    pub authority_source: String,
    pub version: i64,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PaletteEntry {
    pub category: String,
    pub color_id: String,
    pub origin: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct InversePaletteEntry {
    pub color_id: String,
    pub categories: Vec<String>,
    pub status: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsResponse {
    pub schema_version: String,
    pub settings: Vec<SettingSummary>,
    pub palette: Vec<PaletteEntry>,
    pub inverse: Vec<InversePaletteEntry>,
}

#[utoipa::path(get,path="/settings",responses((status=200,body=SettingsResponse)))]
pub async fn list(State(state): State<AppState>) -> Result<Json<SettingsResponse>> {
    Ok(Json(setting_authoring::list(&state).await?))
}
#[utoipa::path(put,path="/setting/{name}",params(("name"=String,Path)),request_body=SettingWriteRequest,
    responses((status=200,body=SettingWriteResponse),(status=400,description="Invalid name, colour or schema version")))]
pub async fn put(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<SettingWriteRequest>,
) -> Result<Json<SettingWriteResponse>> {
    match request.schema_version.as_deref() {
        Some(SETTING_SCHEMA_VERSION) => {}
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
    let (setting_id, version) = setting_authoring::put(&state, &name, request.value).await?;
    Ok(Json(SettingWriteResponse {
        schema_version: SETTING_SCHEMA_VERSION.into(),
        setting_id,
        version,
    }))
}
#[utoipa::path(delete,path="/setting/{name}",params(("name"=String,Path)),responses((status=204,description="Setting removed; fallback applies"),(status=400,description="Unknown namespace"),(status=404,description="No Setting override")))]
pub async fn delete(State(state): State<AppState>, Path(name): Path<String>) -> Result<StatusCode> {
    setting_authoring::delete(&state, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}
