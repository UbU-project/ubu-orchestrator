use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("unsupported proposal operation `{operation}` for candidate kind {candidate_kind:?}")]
    UnsupportedProposal {
        operation: String,
        candidate_kind: ubu_core::CandidateKind,
    },
    #[error("advisory target `{id}` does not exist")]
    TargetNotFound { id: String },
    #[error(transparent)]
    Core(#[from] ubu_core::UbuError),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("{message}")]
    Diagnostic {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("upstream service error: {0}")]
    Upstream(String),
    #[error("store error: {0}")]
    Store(#[from] ubu_store::StoreError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn bad_request_diagnostic(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Diagnostic {
            status: StatusCode::BAD_REQUEST,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn conflict_diagnostic(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Diagnostic {
            status: StatusCode::CONFLICT,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub struct StartupError(pub String);

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "startup error: {}", self.0)
    }
}

impl std::error::Error for StartupError {}

impl StartupError {
    pub fn registration_file(path: &std::path::Path, action: &str, error: impl fmt::Display) -> Self {
        Self(format!("failed to {action} Device registration file `{}`: {error}", path.display()))
    }

    pub fn store_open(e: ubu_store::StoreError) -> Self {
        Self(format!("failed to open store: {e}"))
    }

    pub fn projection_tables(e: sqlx::Error) -> Self {
        Self(format!("failed to initialize projection tables: {e}"))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ErrorBody {
    error: String,
    diagnostics: Vec<ApiDiagnostic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ApiDiagnostic {
    code: String,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::UnsupportedProposal { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TargetNotFound { .. } => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Diagnostic { status, .. } => *status,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::Core(_) | Self::Store(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let diagnostics = match &self {
            Self::UnsupportedProposal { .. } => vec![ApiDiagnostic {
                code: "UnsupportedProposal".into(),
                message: self.to_string(),
            }],
            Self::TargetNotFound { .. } => vec![ApiDiagnostic {
                code: "TargetNotFound".into(),
                message: self.to_string(),
            }],
            Self::Diagnostic { code, message, .. } => vec![ApiDiagnostic {
                code: code.clone(),
                message: message.clone(),
            }],
            _ => Vec::new(),
        };

        let body = Json(ErrorBody {
            error: self.to_string(),
            diagnostics,
        });
        (status, body).into_response()
    }
}
