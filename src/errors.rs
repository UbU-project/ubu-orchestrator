use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
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
    pub fn store_open(e: ubu_store::StoreError) -> Self {
        Self(format!("failed to open store: {e}"))
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
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Diagnostic { status, .. } => *status,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::Store(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let diagnostics = match &self {
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
