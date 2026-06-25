use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::core::UniverseState;
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp};
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

use crate::api::bootstrap::{
    AttentionPreference, BootstrapAnswerRequest, BootstrapAnswerResponse, BootstrapDiagnostic,
    BootstrapSeedRequest, BootstrapSeedResponse, BootstrapStartResponse, WorkStyle,
    BOOTSTRAP_SCHEMA_VERSION,
};
use crate::api::github::ImportLiveRequest;
use crate::errors::{AppError, Result};
use crate::services::import_service;
use crate::state::AppState;

pub async fn start(state: AppState) -> Result<BootstrapStartResponse> {
    let mut started = state.inner().bootstrap_started.lock().await;
    *started = true;
    Ok(BootstrapStartResponse {
        started: true,
        next_prompt: "import_github_fixture".to_owned(),
    })
}

pub async fn answer(
    state: AppState,
    request: BootstrapAnswerRequest,
) -> Result<BootstrapAnswerResponse> {
    let mut answers = state.inner().bootstrap_answers.lock().await;
    answers.push(request.answer);
    Ok(BootstrapAnswerResponse {
        accepted: true,
        answer_count: answers.len(),
    })
}

pub async fn seed(state: AppState, request: BootstrapSeedRequest) -> Result<BootstrapSeedResponse> {
    validate_schema_version(request.schema_version.as_deref())?;
    validate_answers(&request)?;
    reject_if_already_seeded(&state).await?;
    ensure_github_token_available(&state).await?;

    let pool = state.inner().store.pool();
    let objective_id = admit_objective(&state, &request).await?;
    let preference_ids = admit_preferences(pool, &request).await?;

    let imported_tasks = import_service::import_live(
        state.clone(),
        ImportLiveRequest {
            owner: request.selected_repo.owner.clone(),
            repo: request.selected_repo.repo.clone(),
            session_token: None,
            objective_id: Some(objective_id.clone()),
        },
    )
    .await?;
    let universe_state_id = admit_universe_state(pool, &request).await?;

    Ok(BootstrapSeedResponse {
        schema_version: BOOTSTRAP_SCHEMA_VERSION.to_owned(),
        objective_ids: vec![objective_id],
        preference_ids,
        universe_state_id,
        imported_tasks,
        diagnostics: vec![BootstrapDiagnostic {
            code: "bootstrap_seeded".to_owned(),
            message: "bootstrap state admitted through the store".to_owned(),
        }],
    })
}

async fn ensure_github_token_available(state: &AppState) -> Result<()> {
    let has_token = state.inner().config.developer_github_token().is_some()
        || state.inner().desktop_session_token.lock().await.is_some();
    if has_token {
        return Ok(());
    }

    Err(AppError::bad_request_diagnostic(
        "missing_github_token",
        "bootstrap task import requires GITHUB_TOKEN or a pasted desktop session token",
    ))
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(BOOTSTRAP_SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

fn validate_answers(request: &BootstrapSeedRequest) -> Result<()> {
    if request.selected_repo.owner.trim().is_empty() || request.selected_repo.repo.trim().is_empty()
    {
        return Err(AppError::bad_request_diagnostic(
            "missing_selected_repo",
            "selected_repo.owner and selected_repo.repo are required",
        ));
    }

    if request.answers.primary_objective.trim().is_empty() {
        return Err(AppError::bad_request_diagnostic(
            "missing_primary_objective",
            "answers.primary_objective is required",
        ));
    }

    if !(1..=30).contains(&request.answers.planning_horizon_days) {
        return Err(AppError::bad_request_diagnostic(
            "invalid_planning_horizon_days",
            "answers.planning_horizon_days must be between 1 and 30",
        ));
    }

    Ok(())
}

async fn reject_if_already_seeded(state: &AppState) -> Result<()> {
    let bootstrap_source_needle = r#"%"source_kind":"bootstrap"%"#;
    let row = sqlx::query(
        "SELECT COUNT(*) AS count FROM objects
        WHERE object_type IN (?, ?) AND payload_json LIKE ?",
    )
    .bind(ObjectType::Objective.as_str())
    .bind(ObjectType::Preference.as_str())
    .bind(bootstrap_source_needle)
    .fetch_one(state.inner().store.pool())
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let count: i64 = row
        .try_get("count")
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if count > 0 {
        return Err(AppError::conflict_diagnostic(
            "bootstrap_already_seeded",
            "bootstrap-seeded state already exists; refusing to duplicate objects",
        ));
    }

    Ok(())
}

async fn admit_objective(state: &AppState, request: &BootstrapSeedRequest) -> Result<String> {
    let objective_id = UbuId::new(ObjectType::Objective).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let repo = format!(
        "{}/{}",
        request.selected_repo.owner, request.selected_repo.repo
    );

    let record = NewObjectRecord {
        id: objective_id.clone(),
        object_type: ObjectType::Objective.as_str().to_owned(),
        version: 1,
        status: "active".to_owned(),
        compartment_label: "bootstrap".to_owned(),
        payload: json!({
            "id": objective_id,
            "title": request.answers.primary_objective.trim(),
            "description": format!("Bootstrapped objective for {repo}"),
            "status": "active",
            "priority": 1,
            "provenance": bootstrap_provenance(&now)
        }),
        created_at: now.clone(),
        updated_at: now,
    };

    queries::admit_object(state.inner().store.pool(), record)
        .await
        .map_err(AppError::from)?;

    Ok(objective_id)
}

async fn admit_preferences(
    pool: &sqlx::SqlitePool,
    request: &BootstrapSeedRequest,
) -> Result<Vec<String>> {
    let mut preferences = vec![
        (
            "work_style",
            json!(work_style_value(request.answers.work_style)),
        ),
        (
            "planning_horizon_days",
            json!(request.answers.planning_horizon_days),
        ),
        (
            "attention_preference",
            json!(attention_preference_value(
                request.answers.attention_preference
            )),
        ),
    ];
    if let Some(value) = request.answers.acceptable_energy_floor.as_deref() {
        preferences.push(("acceptable_energy_floor", json!(value.trim())));
    }
    if let Some(value) = request.answers.tolerable_stress_ceiling.as_deref() {
        preferences.push(("tolerable_stress_ceiling", json!(value.trim())));
    }
    if let Some(value) = request.answers.tolerable_intensity_ceiling.as_deref() {
        preferences.push(("tolerable_intensity_ceiling", json!(value.trim())));
    }

    let mut admitted = Vec::with_capacity(preferences.len());
    for (name, value) in preferences {
        admitted.push(admit_preference(pool, name, value).await?);
    }

    Ok(admitted)
}

async fn admit_universe_state(
    pool: &sqlx::SqlitePool,
    request: &BootstrapSeedRequest,
) -> Result<String> {
    let captured_at = UbuTimestamp::now_utc();
    let created_at = captured_at.to_string();
    let mut universe_state =
        UniverseState::new(captured_at, "initial UniverseState from bootstrap answers");
    let universe_state_id = universe_state.id.to_string();
    let repo = format!(
        "{}/{}",
        request.selected_repo.owner, request.selected_repo.repo
    );

    universe_state.facts.insert(
        "facts.operator.work_style".to_owned(),
        json!(work_style_value(request.answers.work_style)),
    );
    universe_state.facts.insert(
        "facts.operator.attention_preference".to_owned(),
        json!(attention_preference_value(
            request.answers.attention_preference
        )),
    );
    universe_state
        .facts
        .insert("facts.project.repository".to_owned(), json!(repo));
    universe_state.facts.insert(
        "facts.project.objective".to_owned(),
        json!(request.answers.primary_objective.trim()),
    );
    universe_state.numeric_values.insert(
        "numeric_values.operator.planning_horizon_days".to_owned(),
        f64::from(request.answers.planning_horizon_days),
    );

    let mut payload =
        serde_json::to_value(&universe_state).map_err(|e| AppError::Internal(e.to_string()))?;
    payload["schema_version"] = json!("core/universe-state/0.1");
    payload["set_memberships"] = json!({});
    payload["event_markers"] = json!({});
    payload["provenance"] = bootstrap_provenance(&created_at);

    let record = NewObjectRecord {
        id: universe_state_id.clone(),
        object_type: ObjectType::UniverseState.as_str().to_owned(),
        version: 1,
        status: "active".to_owned(),
        compartment_label: "bootstrap".to_owned(),
        payload,
        created_at: created_at.clone(),
        updated_at: created_at,
    };

    queries::admit_object(pool, record)
        .await
        .map_err(AppError::from)?;

    Ok(universe_state_id)
}

async fn admit_preference(pool: &sqlx::SqlitePool, name: &str, value: Value) -> Result<String> {
    let preference_id = UbuId::new(ObjectType::Preference).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let record = NewObjectRecord {
        id: preference_id.clone(),
        object_type: ObjectType::Preference.as_str().to_owned(),
        version: 1,
        status: "active".to_owned(),
        compartment_label: "bootstrap".to_owned(),
        payload: json!({
            "id": preference_id,
            "name": name,
            "value": value,
            "authority_source": authority_source_wire(AuthoritySource::User)?,
            "provenance": bootstrap_provenance(&now)
        }),
        created_at: now.clone(),
        updated_at: now,
    };

    queries::admit_object(pool, record)
        .await
        .map_err(AppError::from)?;

    Ok(preference_id)
}

fn bootstrap_provenance(created_at: &str) -> Value {
    json!({
        "created_at": created_at,
        "authority_source": "user",
        "source": {
            "source_kind": "bootstrap",
            "source_id": BOOTSTRAP_SCHEMA_VERSION
        }
    })
}

fn authority_source_wire(authority_source: AuthoritySource) -> Result<String> {
    let serialized =
        serde_json::to_string(&authority_source).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(serialized.trim_matches('"').to_owned())
}

fn work_style_value(value: WorkStyle) -> &'static str {
    match value {
        WorkStyle::Focused => "focused",
        WorkStyle::Balanced => "balanced",
        WorkStyle::Responsive => "responsive",
    }
}

fn attention_preference_value(value: AttentionPreference) -> &'static str {
    match value {
        AttentionPreference::DeepWork => "deep_work",
        AttentionPreference::Mixed => "mixed",
        AttentionPreference::QuickTurnaround => "quick_turnaround",
    }
}
