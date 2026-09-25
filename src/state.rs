use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use ubu_core::{
    AuthoritySource, CausalityIssuer, DeviceRegistration, EnvelopeRequest, LocalIssuer,
    MutationEnvelope, UbuId, UbuTimestamp, VersionRef,
};
use ubu_store::UbuStore;

use crate::category_palette::CategoryPalette;
use crate::config::{PlannerStrategyChoice, SecretToken, ServerConfig};
use crate::device_registration::{load_or_register, new_registration, require_registered};
use crate::errors::StartupError;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<OrchestratorState>,
    clock: Arc<dyn crate::planning_time::PlanningClock>,
    calendar_api: Option<Arc<dyn crate::services::calendar_client::CalendarApi>>,
}

pub struct OrchestratorState {
    pub config: ServerConfig,
    pub planning_horizon_seconds: u64,
    pub planner_strategy: PlannerStrategyChoice,
    pub category_palette: CategoryPalette,
    pub store: UbuStore,
    pub device_registration: DeviceRegistration,
    pub causality_issuer: LocalIssuer,
    pub desktop_session_token: Mutex<Option<SecretToken>>,
    pub google_calendar_enabled: std::sync::atomic::AtomicBool,
    pub bootstrap_started: Mutex<bool>,
    pub quick_ubu_import_lock: Mutex<()>,
    pub routine_materialization_lock: Mutex<()>,
    pub calendar_projection_lock: Mutex<()>,
    pub bootstrap_answers: Mutex<Vec<String>>,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> Result<Self, StartupError> {
        // Refuse invalid or revoked operator material before touching the database.
        let span = config.planning_horizon_seconds()?;
        let strategy = config.planner_strategy()?;
        let palette = CategoryPalette::load(config.category_palette_path())?;
        let registration = load_or_register(&config.device_registration_path())?;
        let store = UbuStore::connect(config.db_path())
            .await
            .map_err(StartupError::store_open)?;
        Self::from_store(config, store, registration, palette, span, strategy).await
    }

    /// Isolated convenience constructor: a fresh ephemeral registration, no registration file I/O.
    pub async fn in_memory(config: ServerConfig) -> Result<Self, StartupError> {
        Self::in_memory_with_registration(config, new_registration()).await
    }

    /// Tests can supply restored registration material without a registration file.
    pub async fn in_memory_with_registration(
        config: ServerConfig,
        registration: DeviceRegistration,
    ) -> Result<Self, StartupError> {
        let span = config.planning_horizon_seconds()?;
        let strategy = config.planner_strategy()?;
        let palette = CategoryPalette::load(config.category_palette_path())?;
        require_registered(&registration)?;
        let store = UbuStore::in_memory()
            .await
            .map_err(StartupError::store_open)?;
        Self::from_store(config, store, registration, palette, span, strategy).await
    }

    async fn from_store(
        config: ServerConfig,
        store: UbuStore,
        registration: DeviceRegistration,
        category_palette: CategoryPalette,
        planning_horizon_seconds: u64,
        planner_strategy: PlannerStrategyChoice,
    ) -> Result<Self, StartupError> {
        require_registered(&registration)?;
        ensure_orchestrator_projection_tables(store.pool())
            .await
            .map_err(StartupError::projection_tables)?;
        let causality_issuer = LocalIssuer::new(registration.device_id.clone());
        Ok(Self {
            clock: Arc::new(crate::planning_time::SystemClock),
            calendar_api: None,
            inner: Arc::new(OrchestratorState {
                config,
                category_palette,
                planning_horizon_seconds,
                planner_strategy,
                store,
                device_registration: registration,
                causality_issuer,
                desktop_session_token: Mutex::new(None),
                google_calendar_enabled: std::sync::atomic::AtomicBool::new(false),
                bootstrap_started: Mutex::new(false),
                quick_ubu_import_lock: Mutex::new(()),
                routine_materialization_lock: Mutex::new(()),
                calendar_projection_lock: Mutex::new(()),
                bootstrap_answers: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn with_clock(mut self, clock: impl crate::planning_time::PlanningClock + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub fn planning_now(&self) -> UbuTimestamp {
        self.clock.now()
    }

    pub fn with_calendar_api(mut self, api: Arc<dyn crate::services::calendar_client::CalendarApi>) -> Self {
        self.calendar_api = Some(api);
        self
    }

    pub fn calendar_api(&self) -> Option<Arc<dyn crate::services::calendar_client::CalendarApi>> {
        self.calendar_api.clone()
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
