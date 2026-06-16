use std::fs;

use serde::Deserialize;
use serde_json::json;
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp};
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

use crate::api::github::{
    ImportFixtureRequest, ImportLiveRequest, ImportResponse, ImportedCandidate,
};
use crate::config::SecretToken;
use crate::errors::{AppError, Result};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct FixtureFile {
    candidates: Vec<RawCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RawCandidate {
    pub title: String,
    pub source: String,
}

async fn admit_task(
    pool: &sqlx::SqlitePool,
    title: &str,
    source: &str,
    authority_source: AuthoritySource,
    provenance_source_kind: &str,
    provenance_source_id: &str,
    provenance_source_url: Option<String>,
) -> Result<ImportedCandidate> {
    let task_id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let authority_str =
        serde_json::to_string(&authority_source).map_err(|e| AppError::Internal(e.to_string()))?;
    let authority_str = authority_str.trim_matches('"');

    let record = NewObjectRecord {
        id: task_id.clone(),
        object_type: ObjectType::Task.as_str().to_owned(),
        version: 1,
        status: "active".to_owned(),
        compartment_label: "github-import".to_owned(),
        payload: json!({
            "id": task_id,
            "title": title,
            "status": "active",
            "provenance": {
                "created_at": now,
                "authority_source": authority_str,
                "source": {
                    "source_kind": provenance_source_kind,
                    "source_id": provenance_source_id,
                    "url": provenance_source_url
                }
            }
        }),
        created_at: now.clone(),
        updated_at: now,
    };

    queries::admit_object(pool, record)
        .await
        .map_err(AppError::from)?;

    Ok(ImportedCandidate {
        task_id,
        title: title.to_owned(),
        source: source.to_owned(),
    })
}

pub async fn import_fixture(
    state: AppState,
    request: ImportFixtureRequest,
) -> Result<ImportResponse> {
    let content = fs::read_to_string(&request.fixture_path)
        .map_err(|e| AppError::BadRequest(format!("failed to read fixture: {e}")))?;
    let fixture: FixtureFile = serde_json::from_str(&content)
        .map_err(|e| AppError::BadRequest(format!("failed to parse fixture: {e}")))?;

    let pool = state.inner().store.pool();
    let mut admitted = Vec::with_capacity(fixture.candidates.len());
    for raw in &fixture.candidates {
        let candidate = admit_task(
            pool,
            &raw.title,
            &raw.source,
            AuthoritySource::System,
            "github_fixture",
            &raw.source,
            None,
        )
        .await?;
        admitted.push(candidate);
    }

    let count = admitted.len();
    Ok(ImportResponse {
        imported: fixture.candidates.len(),
        admitted_to_store: count,
        candidates: admitted,
    })
}

pub async fn import_live(state: AppState, request: ImportLiveRequest) -> Result<ImportResponse> {
    // Desktop session tokens are accepted from the UI and kept in process memory
    // only. Developer mode continues to use GITHUB_TOKEN from ServerConfig.
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

    let pool = state.inner().store.pool();
    let source_id = format!("{}/{}", request.owner, request.repo);
    let source_url = format!("https://github.com/{source_id}");
    let title = format!(
        "Import live GitHub state for {}/{}",
        request.owner, request.repo
    );
    let candidate = admit_task(
        pool,
        &title,
        "github_live_stub",
        AuthoritySource::System,
        "github_repository",
        &source_id,
        Some(source_url),
    )
    .await?;

    Ok(ImportResponse {
        imported: 1,
        admitted_to_store: 1,
        candidates: vec![candidate],
    })
}
