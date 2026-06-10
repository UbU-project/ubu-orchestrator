use ubu_core::id_registry::ObjectType;
use ubu_core::projection::approval::ProjectionApproval;
use ubu_core::{UbuId, UbuTimestamp};

use crate::adapters::github_adapter::{InMemoryProjectionWriter, ProjectionWriteAdapter};
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
    let preview = ProjectionPreviewResponse {
        preview_id: UbuId::new(ObjectType::ProjectionPreview).to_string(),
        operations: vec!["summarize_next_action".to_owned()],
        requires_approval: true,
    };

    let mut memory = state.inner().memory.lock().await;
    memory.projection_preview = Some(preview.clone());
    Ok(preview)
}

pub async fn approve(
    state: AppState,
    request: ProjectionApproveRequest,
) -> Result<ProjectionResultResponse> {
    let preview_id = UbuId::parse(&request.preview_id)
        .map_err(|error| AppError::BadRequest(format!("invalid preview id: {error}")))?;
    let approval = ProjectionApproval {
        preview_id,
        approved: true,
        approved_at: UbuTimestamp::now_utc(),
        authority_source: request.authority_source.into(),
    };

    let preview = {
        let memory = state.inner().memory.lock().await;
        memory
            .projection_preview
            .clone()
            .ok_or_else(|| AppError::NotFound("no projection preview available".to_owned()))?
    };

    if preview.preview_id != request.preview_id {
        return Err(AppError::BadRequest(
            "approval preview_id does not match current preview".to_owned(),
        ));
    }

    let writer = InMemoryProjectionWriter;
    let result = writer.apply_approval(&preview, approval)?;

    let mut memory = state.inner().memory.lock().await;
    memory.projection_result = Some(result.clone());
    Ok(result)
}
