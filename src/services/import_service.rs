use std::fs;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use ubu_core::core::{ExternalReference, Task};
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp};
use ubu_github_adapter::auth::GitHubAuth;
use ubu_github_adapter::candidate_mapping::map_repository_state;
use ubu_github_adapter::cli::import_live::import_live_repository;
use ubu_github_adapter::client::{GitHubClient, RecordingGitHubApi};
use ubu_github_adapter::errors::AdapterError;
use ubu_github_adapter::fixture::GitHubFixture;
use ubu_github_adapter::sources::GitHubRepositorySource;
use ubu_store::models::external_reference_record::NewExternalReferenceRecord;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

use crate::api::github::{
    ImportFixtureRequest, ImportLiveRequest, ImportResponse, ImportedCandidate,
};
use crate::config::GithubIngestMode;
use crate::errors::{AppError, Result};
use crate::state::AppState;

const MOCK_GITHUB_ISSUES_FIXTURE: &str = include_str!("../../fixtures/github/issues-small.json");

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
    description: Option<&str>,
    status: &str,
    source: &str,
    authority_source: AuthoritySource,
    provenance_source_kind: &str,
    provenance_source_id: &str,
    provenance_source_url: Option<String>,
    objective_id: Option<String>,
) -> Result<ImportedCandidate> {
    let task_id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let authority_str =
        serde_json::to_string(&authority_source).map_err(|e| AppError::Internal(e.to_string()))?;
    let authority_str = authority_str.trim_matches('"');

    let mut payload = json!({
        "id": task_id.clone(),
        "title": title,
        "status": status,
        "provenance": {
            "created_at": now,
            "authority_source": authority_str,
            "source": {
                "source_kind": provenance_source_kind,
                "source_id": provenance_source_id,
                "url": provenance_source_url
            }
        }
    });
    if let Some(description) = description {
        payload["description"] = json!(description);
    }
    if let Some(objective_id) = objective_id {
        payload["objective_id"] = json!(objective_id);
    }

    let record = NewObjectRecord {
        id: task_id.clone(),
        object_type: ObjectType::Task.as_str().to_owned(),
        version: 1,
        status: "active".to_owned(),
        compartment_label: "github-import".to_owned(),
        payload,
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
            None,
            "active",
            &raw.source,
            AuthoritySource::System,
            "github_fixture",
            &raw.source,
            None,
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
    let client = github_ingest_client(&state, &request.owner, &request.repo).await?;
    let normalized = import_live_repository(&client, &request.owner, &request.repo)
        .await
        .map_err(adapter_app_error)?;
    let mapping = map_repository_state(&normalized).map_err(adapter_app_error)?;
    let pool = state.inner().store.pool();

    let mut admitted = Vec::with_capacity(mapping.tasks.len());
    for task in &mapping.tasks {
        admitted.push(admit_mapped_task(pool, task, request.objective_id.clone()).await?);
    }

    for external_reference in &mapping.external_references {
        store_external_reference(pool, external_reference).await?;
    }

    let admitted_to_store = mapping.tasks.len() + mapping.external_references.len();
    Ok(ImportResponse {
        imported: normalized.issues.len(),
        admitted_to_store,
        candidates: admitted,
    })
}

async fn github_ingest_client(state: &AppState, owner: &str, repo: &str) -> Result<GitHubClient> {
    match state.inner().config.github_ingest_mode() {
        GithubIngestMode::Mock => mock_github_client(owner, repo),
        GithubIngestMode::Live => live_github_client(state).await,
    }
}

async fn live_github_client(state: &AppState) -> Result<GitHubClient> {
    let token = state
        .inner()
        .desktop_session_token
        .lock()
        .await
        .clone()
        .or_else(|| state.inner().config.developer_github_token());
    let Some(token) = token else {
        return Err(AppError::bad_request_diagnostic(
            "missing_github_ingest_token",
            "live GitHub ingest requires GITHUB_TOKEN or an in-memory desktop session token",
        ));
    };

    let auth = GitHubAuth::from_session_token(token.expose_for_adapter().to_owned())
        .map_err(adapter_app_error)?;
    GitHubClient::from_auth(auth).map_err(adapter_app_error)
}

fn mock_github_client(owner: &str, repo: &str) -> Result<GitHubClient> {
    let fixture: GitHubFixture = serde_json::from_str(MOCK_GITHUB_ISSUES_FIXTURE)
        .map_err(|e| AppError::Internal(format!("failed to parse mock GitHub fixture: {e}")))?;
    let repository = GitHubRepositorySource {
        owner: owner.to_owned(),
        name: repo.to_owned(),
        default_branch: fixture.repository.default_branch,
        html_url: format!("https://github.com/{owner}/{repo}"),
        api_id: fixture.repository.api_id,
    };
    let repository_name = repository.full_name();
    let repository_url = repository.html_url.clone();
    let issues = fixture.issues.into_iter().map(|mut issue| {
        issue.repository = repository_name.clone();
        issue.html_url = format!("{repository_url}/issues/{}", issue.number);
        issue
    });

    let api = Arc::new(RecordingGitHubApi::with_repository(repository));
    api.seed_issues(owner, repo, issues);
    Ok(GitHubClient::from_api(api))
}

async fn admit_mapped_task(
    pool: &sqlx::SqlitePool,
    task: &Task,
    objective_id: Option<String>,
) -> Result<ImportedCandidate> {
    let source_ref = task
        .provenance
        .source_refs
        .as_ref()
        .and_then(|refs| refs.first());
    let Some(source_ref) = source_ref else {
        return Err(AppError::Internal(format!(
            "mapped GitHub task `{}` is missing a source reference",
            task.id
        )));
    };

    admit_task(
        pool,
        &task.title,
        task.description.as_deref(),
        task.status.as_str(),
        &source_ref.source_kind,
        task.provenance.authority_source,
        &source_ref.source_kind,
        &source_ref.source_id,
        source_ref.url.clone(),
        objective_id,
    )
    .await
}

async fn store_external_reference(
    pool: &sqlx::SqlitePool,
    external_reference: &ExternalReference,
) -> Result<()> {
    queries::store_external_reference(
        pool,
        NewExternalReferenceRecord {
            id: external_reference.id.to_string(),
            source_type: external_reference.source.source_kind.clone(),
            source_id: external_reference.source.source_id.clone(),
            url: external_reference.source.url.clone(),
            payload_hash: None,
            payload: serde_json::to_value(external_reference)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: external_reference.observed_at.to_string(),
        },
    )
    .await
    .map_err(AppError::from)?;
    Ok(())
}

fn adapter_app_error(error: AdapterError) -> AppError {
    if error.is_rate_limit_or_transport_failure() {
        AppError::Upstream(error.to_string())
    } else {
        AppError::Internal(error.to_string())
    }
}
