use std::sync::Arc;

use tokio::sync::Mutex;
use ubu_planning_core::{Plan, PlanningRequest, PlanningResponse};

use crate::api::github::ImportedCandidate;
use crate::api::next_action::NextActionResponse;
use crate::api::projection::{ProjectionPreviewResponse, ProjectionResultResponse};
use crate::api::user_action::LogEntryResponse;
use crate::config::{SecretToken, ServerConfig};

#[derive(Clone)]
pub struct AppState {
    inner: Arc<OrchestratorState>,
}

pub struct OrchestratorState {
    pub config: ServerConfig,
    pub memory: Mutex<MemoryState>,
    pub desktop_session_token: Mutex<Option<SecretToken>>,
}

#[derive(Debug, Default)]
pub struct MemoryState {
    pub bootstrap_started: bool,
    pub bootstrap_answers: Vec<String>,
    pub imported_candidates: Vec<ImportedCandidate>,
    pub planning_request: Option<PlanningRequest>,
    pub planning_response: Option<PlanningResponse>,
    pub admitted_plan: Option<Plan>,
    pub next_action: Option<NextActionResponse>,
    pub log_entries: Vec<LogEntryResponse>,
    pub projection_preview: Option<ProjectionPreviewResponse>,
    pub projection_result: Option<ProjectionResultResponse>,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            inner: Arc::new(OrchestratorState {
                config,
                memory: Mutex::new(MemoryState::default()),
                desktop_session_token: Mutex::new(None),
            }),
        }
    }

    pub fn inner(&self) -> &Arc<OrchestratorState> {
        &self.inner
    }
}
