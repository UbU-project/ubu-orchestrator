use std::fs;

use serde::Deserialize;

use crate::adapters::store_adapter::{InMemoryStoreAdapter, StoreAdapter};
use crate::api::github::{
    ImportFixtureRequest, ImportLiveRequest, ImportResponse, ImportedCandidate,
};
use crate::config::SecretToken;
use crate::errors::{AppError, Result};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureFile {
    candidates: Vec<ImportedCandidate>,
}

pub async fn import_fixture(
    state: AppState,
    request: ImportFixtureRequest,
) -> Result<ImportResponse> {
    let content = fs::read_to_string(&request.fixture_path)
        .map_err(|error| AppError::BadRequest(format!("failed to read fixture: {error}")))?;
    let fixture: FixtureFile = serde_json::from_str(&content)
        .map_err(|error| AppError::BadRequest(format!("failed to parse fixture: {error}")))?;

    let store = InMemoryStoreAdapter;
    let admitted_to_store = store.admit_candidates(&fixture.candidates)?;

    let mut memory = state.inner().memory.lock().await;
    memory.imported_candidates = fixture.candidates.clone();
    Ok(ImportResponse {
        imported: fixture.candidates.len(),
        admitted_to_store,
        candidates: fixture.candidates,
    })
}

pub async fn import_live(state: AppState, request: ImportLiveRequest) -> Result<ImportResponse> {
    if let Some(token) = SecretToken::new(request.session_token.unwrap_or_default()) {
        let mut session = state.inner().desktop_session_token.lock().await;
        *session = Some(token);
    }

    let has_token = state.inner().config.developer_github_token().is_some()
        || state.inner().desktop_session_token.lock().await.is_some();
    if !has_token {
        return Err(AppError::BadRequest(
            "live import requires GITHUB_TOKEN or a pasted session token".to_owned(),
        ));
    }

    let candidates = vec![ImportedCandidate {
        task_id: format!("{}#live-import", request.repo),
        title: format!(
            "Import live GitHub state for {}/{}",
            request.owner, request.repo
        ),
        source: "github_live_stub".to_owned(),
    }];

    let store = InMemoryStoreAdapter;
    let admitted_to_store = store.admit_candidates(&candidates)?;

    let mut memory = state.inner().memory.lock().await;
    memory.imported_candidates = candidates.clone();
    Ok(ImportResponse {
        imported: candidates.len(),
        admitted_to_store,
        candidates,
    })
}
