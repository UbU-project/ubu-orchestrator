use serde_json::json;
use sqlx::Row;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_store::models::projection_record::{NewProjectionPreviewRecord, NewProjectionResultRecord};
use ubu_store::queries;

use crate::api::projection::{
    ProjectionApproveRequest, ProjectionPreviewRequest, ProjectionPreviewResponse,
    ProjectionResultResponse,
};
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn preview(
    state: AppState,
    _request: ProjectionPreviewRequest,
) -> Result<ProjectionPreviewResponse> {
    let preview_id = UbuId::new(ObjectType::ProjectionPreview).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let operations = vec!["summarize_next_action".to_owned()];

    queries::store_projection_preview(
        state.inner().store.pool(),
        NewProjectionPreviewRecord {
            id: preview_id.clone(),
            request_id: preview_id.clone(),
            status: "pending".to_owned(),
            payload: json!({
                "operations": operations,
                "requires_approval": true,
            }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    Ok(ProjectionPreviewResponse {
        preview_id,
        operations,
        requires_approval: true,
    })
}

pub async fn approve(
    state: AppState,
    request: ProjectionApproveRequest,
) -> Result<ProjectionResultResponse> {
    UbuId::parse(&request.preview_id)
        .map_err(|e| AppError::BadRequest(format!("invalid preview id: {e}")))?;

    let pool = state.inner().store.pool();

    let row = sqlx::query(
        "SELECT id, payload_json FROM projection_previews WHERE id = ?",
    )
    .bind(&request.preview_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .ok_or_else(|| AppError::NotFound("no projection preview available".to_owned()))?;

    let stored_id: String = row
        .try_get("id")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let payload: serde_json::Value = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let operations: Vec<String> =
        serde_json::from_value(payload["operations"].clone()).unwrap_or_default();

    let operation_results: Vec<String> = operations
        .iter()
        .map(|op| format!("{op}:applied"))
        .collect();

    let result_id = UbuId::new(ObjectType::Snapshot).to_string();
    let now = UbuTimestamp::now_utc().to_string();

    queries::store_projection_result(
        pool,
        NewProjectionResultRecord {
            id: result_id,
            preview_id: stored_id.clone(),
            status: "applied".to_owned(),
            payload: json!({ "operation_results": operation_results }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    Ok(ProjectionResultResponse {
        preview_id: stored_id,
        status: "applied".to_owned(),
        operation_results,
    })
}
