use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use ubu_planning_core::{PlanningRequest, PlanningResponse, TaskSpec, PLANNING_SCHEMA_VERSION};
use utoipa::ToSchema;

use crate::errors::Result;
use crate::services::planning_service;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePlanningRequest {
    #[serde(default)]
    pub request: Option<PlanningRequestBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanningRequestBody {
    #[serde(default)]
    pub schema_version: Option<String>,
    pub request_id: String,
    #[serde(default)]
    pub tasks: Vec<TaskSpecBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpecBody {
    pub id: String,
    pub duration: u64,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub affect_required: bool,
    #[serde(default)]
    pub affect_current: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanningResponseBody {
    pub request_id: String,
    pub plan: Option<PlanBody>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanBody {
    pub plan_id: String,
    pub status: String,
    pub tasks: Vec<ScheduledTaskBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskBody {
    pub task_id: String,
    pub start: u64,
    pub end: u64,
    pub depends_on: Vec<String>,
    pub static_anchor: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
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
                    window: None,
                    static_anchor: None,
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
            request_id: value.request_id,
            plan: value.plan.map(|plan| PlanBody {
                plan_id: plan.plan_id,
                status: format!("{:?}", plan.status).to_ascii_lowercase(),
                tasks: plan
                    .tasks
                    .into_iter()
                    .map(|task| ScheduledTaskBody {
                        task_id: task.task_id,
                        start: task.start,
                        end: task.end,
                        depends_on: task.depends_on,
                        static_anchor: task.static_anchor,
                    })
                    .collect(),
            }),
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
