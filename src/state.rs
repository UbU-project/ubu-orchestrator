use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use ubu_core::{
    AuthoritySource, CausalityIssuer, DeviceRegistration, EnvelopeRequest, LocalIssuer,
    MutationEnvelope, UbuId, UbuTimestamp, VersionRef,
};
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

    /// Assemble provenance at the mutation boundary; domain time remains independent.
    pub fn envelope_for(
        &self,
        observed: BTreeMap<UbuId, VersionRef>,
        authority: AuthoritySource,
        effective_time: UbuTimestamp,
    ) -> crate::errors::Result<MutationEnvelope> {
        Ok(self.inner.causality_issuer.issue(EnvelopeRequest {
            observed_versions: observed,
            actor_identity_id: self.actor_identity_id().clone(),
            authority_source: authority,
            effective_time,
            observed_policy_versions: None,
            execution_context: None,
        })?)
    }

    /// Assemble candidate-storage provenance with a caller-supplied deterministic
    /// key. Advisory retries must reuse the candidate's own idempotency key.
    pub fn envelope_with_key(
        &self,
        observed: BTreeMap<ubu_core::UbuId, ubu_core::VersionRef>,
        authority: ubu_core::AuthoritySource,
        effective_time: ubu_core::UbuTimestamp,
        idempotency_key: ubu_core::IdempotencyKey,
    ) -> crate::errors::Result<ubu_core::MutationEnvelope> {
        let now = ubu_core::UbuTimestamp::now_utc();
        let envelope = ubu_core::MutationEnvelope {
            idempotency_key,
            observed_versions: observed,
            origin_device_id: self.inner.device_registration.device_id.clone(),
            actor_identity_id: self.actor_identity_id().clone(),
            authority_source: authority,
            created_time: now,
            effective_time,
            recorded_time: now,
            observed_policy_versions: None,
            execution_context: None,
        };
        envelope.validate()?;
        Ok(envelope)
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
