use std::sync::Arc;

use tokio::sync::Mutex;
use ubu_core::{DeviceRegistration, LocalIssuer, UbuId};
use ubu_store::UbuStore;

use crate::config::{SecretToken, ServerConfig};
use crate::device_registration::{load_or_register, new_registration, require_registered};
use crate::errors::StartupError;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<OrchestratorState>,
}

pub struct OrchestratorState {
    pub config: ServerConfig,
    pub store: UbuStore,
    pub device_registration: DeviceRegistration,
    pub causality_issuer: LocalIssuer,
    pub desktop_session_token: Mutex<Option<SecretToken>>,
    pub bootstrap_started: Mutex<bool>,
    pub bootstrap_answers: Mutex<Vec<String>>,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> Result<Self, StartupError> {
        // Refuse invalid or revoked operator material before touching the database.
        let registration = load_or_register(&config.device_registration_path())?;
        let store = UbuStore::connect(config.db_path())
            .await
            .map_err(StartupError::store_open)?;
        Self::from_store(config, store, registration).await
    }

    /// Isolated convenience constructor: a fresh ephemeral registration, no file I/O.
    pub async fn in_memory(config: ServerConfig) -> Result<Self, StartupError> {
        Self::in_memory_with_registration(config, new_registration()).await
    }

    /// Tests can supply restored registration material without any real file.
    pub async fn in_memory_with_registration(
        config: ServerConfig,
        registration: DeviceRegistration,
    ) -> Result<Self, StartupError> {
        require_registered(&registration)?;
        let store = UbuStore::in_memory()
            .await
            .map_err(StartupError::store_open)?;
        Self::from_store(config, store, registration).await
    }

    async fn from_store(
        config: ServerConfig,
        store: UbuStore,
        registration: DeviceRegistration,
    ) -> Result<Self, StartupError> {
        require_registered(&registration)?;
        ensure_orchestrator_projection_tables(store.pool())
            .await
            .map_err(StartupError::projection_tables)?;
        let causality_issuer = LocalIssuer::new(registration.device_id.clone());
        Ok(Self {
            inner: Arc::new(OrchestratorState {
                config,
                store,
                device_registration: registration,
                causality_issuer,
                desktop_session_token: Mutex::new(None),
                bootstrap_started: Mutex::new(false),
                bootstrap_answers: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn actor_identity_id(&self) -> &UbuId {
        &self.inner.device_registration.registered_identity_id
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
