use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use ubu_planning_core::{
    PlanningRequest, PlanningResponse, StaticAnchor, TaskSpec, TimeWindow, PLANNING_SCHEMA_VERSION,
};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::planning_service;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct GeneratePlanningRequest {
    #[serde(default)]
    pub schema_version: Option<String>,
    #[serde(default)]
    pub request: Option<PlanningRequestBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlanningRequestBody {
    #[serde(default)]
    pub schema_version: Option<String>,
    pub request_id: String,
    #[serde(default = "default_planning_mode")]
    pub mode: PlanningModeBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_window: Option<TimeWindowBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_graph: Option<TaskGraphBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_context: Option<RepairContextBody>,
    #[serde(default)]
    pub tasks: Vec<TaskSpecBody>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningModeBody {
    FreshGeneration,
    Repair,
}

fn default_planning_mode() -> PlanningModeBody {
    PlanningModeBody::FreshGeneration
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TimeWindowBody {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TaskGraphBody {
    pub topological_order: Vec<String>,
    #[serde(default)]
    pub edges: Vec<TaskGraphEdgeBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TaskGraphEdgeBody {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct RepairContextBody {
    pub prior_plan_id: String,
    pub last_legitimate_plan_ref: String,
    #[serde(default)]
    pub observed_divergence_refs: Vec<String>,
    pub repair_scope: RepairScopeBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepairScopeBody {
    RemainingWindow,
    FailedTask,
    MootTask,
    OverridePlacement,
    FullWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct TaskSpecBody {
    pub id: String,
    pub duration: u64,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub affect_required: bool,
    #[serde(default)]
    pub affect_current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindowBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_anchor: Option<StaticAnchorBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct StaticAnchorBody {
    pub start: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlanningResponseBody {
    pub schema_version: String,
    pub request_id: String,
    pub plan: Option<PlanBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlanBody {
    pub id: String,
    pub status: String,
    pub steps: Vec<ScheduledTaskBody>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_plan_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct ScheduledTaskBody {
    pub index: u32,
    pub task_id: String,
    pub summary: String,
    pub start: u64,
    pub end: u64,
    pub depends_on: Vec<String>,
    pub static_anchor: bool,
    pub placement_authority: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct DiagnosticBody {
    pub code: String,
    pub message: String,
}

impl From<PlanningRequestBody> for PlanningRequest {
    fn from(value: PlanningRequestBody) -> Self {
        Self {
            schema_version: Some(
                value
                    .schema_version
                    .unwrap_or_else(|| PLANNING_SCHEMA_VERSION.to_owned()),
            ),
            request_id: value.request_id,
            tasks: value
                .tasks
                .into_iter()
                .map(|task| TaskSpec {
                    id: task.id,
                    duration: task.duration,
                    depends_on: task.depends_on,
                    window: task.window.map(|window| TimeWindow {
                        start: window.start,
                        end: window.end,
                    }),
                    static_anchor: task.static_anchor.map(|anchor| StaticAnchor {
                        start: anchor.start,
                    }),
                    affect_required: task.affect_required,
                    affect_current: task.affect_current,
                })
                .collect(),
        }
    }
}

impl From<PlanningResponse> for PlanningResponseBody {
    fn from(value: PlanningResponse) -> Self {
        Self {
            schema_version: value.schema_version,
            request_id: value.request_id,
            plan: value
                .plan
                .map(crate::services::planning_service::kernel_plan_body),
            diagnostics: value
                .diagnostics
                .into_iter()
                .map(|diagnostic| DiagnosticBody {
                    code: format!("{:?}", diagnostic.code),
                    message: diagnostic.message,
                })
                .collect(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/planning/generate",
    request_body = GeneratePlanningRequest,
    responses((status = 200, body = PlanningResponseBody))
)]
pub async fn generate(
    State(state): State<AppState>,
    Json(request): Json<GeneratePlanningRequest>,
) -> Result<Json<PlanningResponseBody>> {
    let response = planning_service::generate(state, request).await?;
    Ok(Json(response))
}
