use crate::api::desktop::{
    GithubTokenIntakeRequest, GithubTokenIntakeResponse, DESKTOP_SESSION_SCHEMA_VERSION,
};
use crate::config::SecretToken;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn github_token(
    state: AppState,
    request: GithubTokenIntakeRequest,
) -> Result<GithubTokenIntakeResponse> {
    validate_schema_version(request.schema_version.as_deref())?;
    let token = SecretToken::new(request.github_token).ok_or_else(|| {
        AppError::bad_request_diagnostic(
            "missing_github_token",
            "github_token must be provided for desktop session intake",
        )
    })?;

    let mut session = state.inner().desktop_session_token.lock().await;
    *session = Some(token);

    Ok(GithubTokenIntakeResponse {
        schema_version: DESKTOP_SESSION_SCHEMA_VERSION.to_owned(),
        accepted: true,
        token_available: true,
    })
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(DESKTOP_SESSION_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

pub async fn google_calendar(
    state: AppState,
    request: crate::api::desktop::GoogleCalendarEnableRequest,
) -> Result<crate::api::desktop::GoogleCalendarEnableResponse> {
    validate_schema_version(request.schema_version.as_deref())?;
    super::calendar_client::require_live_configuration(&state)?;
    state.inner().google_calendar_enabled.store(true, std::sync::atomic::Ordering::Release);
    Ok(crate::api::desktop::GoogleCalendarEnableResponse {
        schema_version: DESKTOP_SESSION_SCHEMA_VERSION.into(),
        accepted: true,
        enabled: true,
    })
}
