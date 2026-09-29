//! Explicit producer trigger. Construction of a live transport is executable wiring only.
use crate::services::advisory_wire::SelectedTask;
use crate::{
    api::planning::DiagnosticBody,
    errors::{AppError, Result},
    services::{advisory_service, setting_authoring, suggest_tags},
    state::AppState,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub const ADVISORY_RUN_SCHEMA_VERSION: &str = "ubu.orchestrator.advisory_run.v1";
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdvisoryRunRequest {
    pub schema_version: Option<String>,
    pub producer: String,
    pub limit: Option<usize>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct AdvisoryRunResponse {
    pub schema_version: String,
    pub status: String,
    pub selected: Vec<SelectedTask>,
    pub candidates_enqueued: usize,
    pub candidate_ids: Vec<String>,
    pub report: Option<Value>,
    pub diagnostics: Vec<DiagnosticBody>,
}

#[utoipa::path(post,path="/advisory/run",request_body=AdvisoryRunRequest,
    responses((status=200,body=AdvisoryRunResponse),(status=400)))]
pub async fn run(
    State(state): State<AppState>,
    Json(request): Json<AdvisoryRunRequest>,
) -> Result<Json<AdvisoryRunResponse>> {
    match request.schema_version.as_deref() {
        Some(ADVISORY_RUN_SCHEMA_VERSION) => {}
        None => {
            return Err(AppError::bad_request_diagnostic(
                "missing_schema_version",
                "schema_version is required",
            ))
        }
        _ => {
            return Err(AppError::bad_request_diagnostic(
                "unknown_schema_version",
                "Unsupported advisory run schema_version",
            ))
        }
    }
    if request.producer != "suggest_tags" {
        return Err(AppError::bad_request_diagnostic(
            "advisory_unknown_producer",
            "Only suggest_tags is implemented",
        ));
    }
    let limit = request.limit.unwrap_or(suggest_tags::DEFAULT_LIMIT);
    if !(1..=suggest_tags::MAX_LIMIT).contains(&limit) {
        return Err(AppError::bad_request_diagnostic(
            "advisory_invalid_limit",
            "limit must be between 1 and 25",
        ));
    }
    let mut response = AdvisoryRunResponse {
        schema_version: ADVISORY_RUN_SCHEMA_VERSION.into(),
        status: "unconfigured".into(),
        selected: vec![],
        candidates_enqueued: 0,
        candidate_ids: vec![],
        report: None,
        diagnostics: vec![],
    };
    let model = setting_authoring::advisory_value(&state, "advisory.model").await?;
    let endpoint = setting_authoring::advisory_value(&state, "advisory.endpoint").await?;
    for (name, missing) in [
        ("advisory.model", model.is_none()),
        ("advisory.endpoint", endpoint.is_none()),
    ] {
        if missing {
            response.diagnostics.push(DiagnosticBody {
                code: "advisory_unconfigured".into(),
                message: format!(
                    "{name} is not configured; set it in Setup before running SuggestTags"
                ),
            });
        }
    }
    if !response.diagnostics.is_empty() {
        return Ok(Json(response));
    }
    let endpoint = endpoint.unwrap();
    if !setting_authoring::valid_advisory_endpoint(&endpoint) {
        response.diagnostics.push(DiagnosticBody {
            code: "advisory_endpoint_invalid".into(),
            message:
                "advisory.endpoint must be a literal loopback HTTP origin; correct it in Setup"
                    .into(),
        });
        return Ok(Json(response));
    }
    // No fallback: an in-memory/test/library state cannot construct a real client.
    let Some(factory) = state.advisory_transport_factory() else {
        response.status = "failed".into();
        response.diagnostics.push(DiagnosticBody {
            code: "advisory_transport_unavailable".into(),
            message:
                "No advisory transport is installed in this process; no candidates were enqueued"
                    .into(),
        });
        return Ok(Json(response));
    };
    response.selected = suggest_tags::select(&state, limit).await?;
    let skipped = suggest_tags::skipped_occurrences(&state).await?;
    if response.selected.is_empty() {
        response.status = "ok".into();
        response.diagnostics = skipped;
        return Ok(Json(response));
    }
    let submission = suggest_tags::submission(&state, &response.selected, &model.unwrap()).await?;
    let transport = factory(&endpoint);
    let runtime = tokio::runtime::Handle::current();
    // The core seam is synchronous. Run the controller off the async server workers.
    let report = tokio::task::spawn_blocking(move || {
        runtime.block_on(advisory_service::run_advisory(
            &state,
            submission,
            transport.as_ref(),
        ))
    })
    .await
    .map_err(|_| AppError::Internal("Advisory controller task failed".into()))??;
    response.status = serde_json::to_value(report.status)
        .expect("status serializes")
        .as_str()
        .unwrap()
        .into();
    response.candidates_enqueued = report.candidates_stored;
    response.candidate_ids = report.candidate_ids.clone();
    // What the model was never asked about comes first, then what it answered.
    response.diagnostics = skipped
        .into_iter()
        .chain(report.diagnostics.iter().filter_map(|value| {
            Some(DiagnosticBody {
                code: value["code"].as_str()?.into(),
                message: value["message"]
                    .as_str()
                    .unwrap_or("Advisory diagnostic")
                    .into(),
            })
        }))
        .collect();
    response.report =
        Some(serde_json::to_value(report).map_err(|e| AppError::Internal(e.to_string()))?);
    Ok(Json(response))
}
