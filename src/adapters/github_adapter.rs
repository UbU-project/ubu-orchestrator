use ubu_core::projection::approval::ProjectionApproval;

use crate::api::projection::{ProjectionPreviewResponse, ProjectionResultResponse};
use crate::errors::{AppError, Result};

pub trait ProjectionWriteAdapter {
    fn apply_approval(
        &self,
        preview: &ProjectionPreviewResponse,
        approval: ProjectionApproval,
    ) -> Result<ProjectionResultResponse>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InMemoryProjectionWriter;

impl ProjectionWriteAdapter for InMemoryProjectionWriter {
    fn apply_approval(
        &self,
        preview: &ProjectionPreviewResponse,
        approval: ProjectionApproval,
    ) -> Result<ProjectionResultResponse> {
        if !approval.approved {
            return Err(AppError::BadRequest(
                "projection write path requires explicit approval".to_owned(),
            ));
        }
        if preview.preview_id != approval.preview_id.to_string() {
            return Err(AppError::BadRequest(
                "projection approval does not target preview".to_owned(),
            ));
        }

        Ok(ProjectionResultResponse {
            preview_id: preview.preview_id.clone(),
            status: "applied".to_owned(),
            operation_results: preview
                .operations
                .iter()
                .map(|operation| format!("{operation}:applied"))
                .collect(),
        })
    }
}
