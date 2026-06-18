use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use ubu_planning_core::{
    AffectDirection, AffectLegitimizationMode, AffectObservation, AffectObservationValue,
    AffectProfile, AffectTolerance, LegitimizationReport, LegitimizationResult, PlanningMode,
    PlanningRequest, PlanningResponse, RepairContext, RepairScope, StaticAnchor, TaskGraph,
    TaskSpec, TimeWindow, PLANNING_SCHEMA_VERSION,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_profile: Option<AffectProfileBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_observation: Option<AffectObservationBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_warning: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AffectLegitimizationModeBody {
    Enforce,
    WarnOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AffectDirectionBody {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AffectToleranceBody {
    pub direction: AffectDirectionBody,
    pub location: f64,
    pub scale: f64,
    pub threshold: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AffectProfileBody {
    #[serde(default = "default_affect_mode")]
    pub mode: AffectLegitimizationModeBody,
    #[serde(default)]
    pub dimensions: BTreeMap<String, AffectToleranceBody>,
}

fn default_affect_mode() -> AffectLegitimizationModeBody {
    AffectLegitimizationModeBody::Enforce
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AffectObservationValueBody {
    pub value: f64,
    pub observed_at: u64,
    pub source_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AffectObservationBody {
    #[serde(default)]
    pub dimensions: BTreeMap<String, AffectObservationValueBody>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlanningResponseBody {
    pub schema_version: String,
    pub request_id: String,
    pub plan: Option<PlanBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legitimization: Option<LegitimizationReportBody>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legitimization: Option<LegitimizationReportBody>,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct LegitimizationReportBody {
    pub result: String,
    pub mode: AffectLegitimizationModeBody,
    pub affect_feasible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affect_margin: Option<f64>,
    #[serde(default)]
    pub violated_dimensions: Vec<String>,
    #[serde(default)]
    pub stale_dimensions: Vec<String>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, AffectDimensionLegitimizationBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_affect_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct AffectDimensionLegitimizationBody {
    pub satisfaction: f64,
    pub threshold: f64,
    pub margin: f64,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct DiagnosticBody {
    pub code: String,
    pub message: String,
}

impl From<PlanningRequestBody> for PlanningRequest {
    fn from(value: PlanningRequestBody) -> Self {
        let tasks = value
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
            })
            .collect::<Vec<_>>();
        let topological_order = value
            .task_graph
            .as_ref()
            .map(|graph| graph.topological_order.clone())
            .unwrap_or_else(|| tasks.iter().map(|task| task.id.clone()).collect());
        Self {
            schema_version: Some(
                value
                    .schema_version
                    .unwrap_or_else(|| PLANNING_SCHEMA_VERSION.to_owned()),
            ),
            request_id: value.request_id,
            mode: planning_mode(value.mode),
            rng_seed: value.rng_seed.unwrap_or_default(),
            time_window: value.time_window.map(|window| TimeWindow {
                start: window.start,
                end: window.end,
            }),
            task_graph: TaskGraph {
                tasks,
                topological_order,
            },
            repair_context: value.repair_context.map(repair_context),
            affect_profile: value.affect_profile.map(affect_profile),
            affect_observation: value.affect_observation.map(affect_observation),
            prior_plan: None,
        }
    }
}

impl From<PlanningResponse> for PlanningResponseBody {
    fn from(value: PlanningResponse) -> Self {
        Self {
            schema_version: value.schema_version,
            request_id: value.request_id,
            legitimization: value
                .legitimization
                .map(|report| legitimization_report_body(report, None)),
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

pub fn legitimization_report_body(
    report: LegitimizationReport,
    stale_affect_warning: Option<String>,
) -> LegitimizationReportBody {
    LegitimizationReportBody {
        result: legitimization_result_wire(report.result).to_owned(),
        mode: affect_mode_body(report.mode),
        affect_feasible: report.affect_feasible,
        affect_margin: report.affect_margin,
        violated_dimensions: report.violated_dimensions,
        stale_dimensions: report.stale_dimensions,
        dimensions: report
            .dimensions
            .into_iter()
            .map(|(dimension, value)| {
                (
                    dimension,
                    AffectDimensionLegitimizationBody {
                        satisfaction: value.satisfaction,
                        threshold: value.threshold,
                        margin: value.margin,
                        stale: value.stale,
                    },
                )
            })
            .collect(),
        stale_affect_warning,
    }
}

fn planning_mode(mode: PlanningModeBody) -> PlanningMode {
    match mode {
        PlanningModeBody::FreshGeneration => PlanningMode::FreshGeneration,
        PlanningModeBody::Repair => PlanningMode::Repair,
    }
}

fn repair_context(value: RepairContextBody) -> RepairContext {
    RepairContext {
        prior_plan_id: value.prior_plan_id,
        last_legitimate_plan_ref: Some(value.last_legitimate_plan_ref),
        observed_divergence_refs: value.observed_divergence_refs,
        repair_scope: repair_scope(value.repair_scope),
    }
}

fn repair_scope(scope: RepairScopeBody) -> RepairScope {
    match scope {
        RepairScopeBody::FullWindow => RepairScope::FullWindow,
        RepairScopeBody::RemainingWindow
        | RepairScopeBody::FailedTask
        | RepairScopeBody::MootTask
        | RepairScopeBody::OverridePlacement => RepairScope::RemainingWindow,
    }
}

fn affect_profile(value: AffectProfileBody) -> AffectProfile {
    AffectProfile {
        mode: affect_mode(value.mode),
        dimensions: value
            .dimensions
            .into_iter()
            .map(|(dimension, tolerance)| {
                (
                    dimension,
                    AffectTolerance {
                        direction: affect_direction(tolerance.direction),
                        location: tolerance.location,
                        scale: tolerance.scale,
                        threshold: tolerance.threshold,
                        freshness_seconds: tolerance.freshness_seconds,
                    },
                )
            })
            .collect(),
    }
}

fn affect_observation(value: AffectObservationBody) -> AffectObservation {
    AffectObservation {
        dimensions: value
            .dimensions
            .into_iter()
            .map(|(dimension, observed)| {
                (
                    dimension,
                    AffectObservationValue {
                        value: observed.value,
                        observed_at: observed.observed_at,
                        source_kind: observed.source_kind,
                    },
                )
            })
            .collect(),
    }
}

fn affect_mode(mode: AffectLegitimizationModeBody) -> AffectLegitimizationMode {
    match mode {
        AffectLegitimizationModeBody::Enforce => AffectLegitimizationMode::Enforce,
        AffectLegitimizationModeBody::WarnOnly => AffectLegitimizationMode::WarnOnly,
    }
}

fn affect_mode_body(mode: AffectLegitimizationMode) -> AffectLegitimizationModeBody {
    match mode {
        AffectLegitimizationMode::Enforce => AffectLegitimizationModeBody::Enforce,
        AffectLegitimizationMode::WarnOnly => AffectLegitimizationModeBody::WarnOnly,
    }
}

fn affect_direction(direction: AffectDirectionBody) -> AffectDirection {
    match direction {
        AffectDirectionBody::HigherIsBetter => AffectDirection::HigherIsBetter,
        AffectDirectionBody::LowerIsBetter => AffectDirection::LowerIsBetter,
    }
}

fn legitimization_result_wire(result: LegitimizationResult) -> &'static str {
    match result {
        LegitimizationResult::Passed => "passed",
        LegitimizationResult::Failed => "failed",
        LegitimizationResult::NeedsClarification => "needs_clarification",
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
