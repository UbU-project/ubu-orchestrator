use std::sync::Arc;

use tokio::sync::Mutex;
use ubu_store::UbuStore;

use crate::config::{SecretToken, ServerConfig};
use crate::errors::StartupError;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<OrchestratorState>,
}

pub struct OrchestratorState {
    pub config: ServerConfig,
    pub store: UbuStore,
    pub desktop_session_token: Mutex<Option<SecretToken>>,
    pub bootstrap_started: Mutex<bool>,
    pub bootstrap_answers: Mutex<Vec<String>>,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> Result<Self, StartupError> {
        let store = UbuStore::connect(config.db_path())
            .await
            .map_err(StartupError::store_open)?;
        ensure_orchestrator_projection_tables(store.pool())
            .await
            .map_err(StartupError::projection_tables)?;
        Ok(Self {
            inner: Arc::new(OrchestratorState {
                config,
                store,
                desktop_session_token: Mutex::new(None),
                bootstrap_started: Mutex::new(false),
                bootstrap_answers: Mutex::new(Vec::new()),
            }),
        })
    }

    pub async fn in_memory(config: ServerConfig) -> Result<Self, StartupError> {
        let store = UbuStore::in_memory()
            .await
            .map_err(StartupError::store_open)?;
        ensure_orchestrator_projection_tables(store.pool())
            .await
            .map_err(StartupError::projection_tables)?;
        Ok(Self {
            inner: Arc::new(OrchestratorState {
                config,
                store,
                desktop_session_token: Mutex::new(None),
                bootstrap_started: Mutex::new(false),
                bootstrap_answers: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn inner(&self) -> &Arc<OrchestratorState> {
        &self.inner
    }
}

async fn ensure_orchestrator_projection_tables(pool: &sqlx::SqlitePool) -> sqlx::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projection_approvals (
            id TEXT PRIMARY KEY,
            preview_id TEXT NOT NULL,
            approved INTEGER NOT NULL,
            authority_source TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            approved_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projection_reconciliations (
            id TEXT PRIMARY KEY,
            preview_id TEXT NOT NULL,
            result_id TEXT NOT NULL,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projection_worker_writes (
            id TEXT PRIMARY KEY,
            preview_id TEXT NOT NULL,
            operation_id TEXT NOT NULL,
            authority_source TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}
