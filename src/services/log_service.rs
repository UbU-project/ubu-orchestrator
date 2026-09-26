use serde_json::json;
use ubu_core::core::{
    apply_universe_mutations, validate_mutations_for_mode, InstanceMode, TaskEffect, UniverseState,
};
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp, VersionRef};
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

use crate::api::user_action::{
    ActionDiagnostic, LogEntryResponse, RecordedTaskActionKind, RecordedTaskActionRequest,
    RecordedTaskActionResponse, TaskActionKind, TaskLifecycleStatus, UserActionRequest,
    TASK_ACTION_SCHEMA_VERSION,
};
use crate::errors::{AppError, Result};
use crate::instance_mode::MVP_INSTANCE_MODE;
use crate::services::{planning_service, recalculation_service};
use crate::state::AppState;

pub async fn record_task_action(
    state: AppState,
    task_id: String,
    request: RecordedTaskActionRequest,
) -> Result<RecordedTaskActionResponse> {
    record_task_action_with_calendar(state, task_id, request, None).await
}

pub async fn record_calendar_completion(
    state: AppState,
    signal: &super::calendar_interaction::CompletionSignal,
) -> Result<RecordedTaskActionResponse> {
    record_task_action_with_calendar(state, signal.task_id.clone(), RecordedTaskActionRequest {
        schema_version: Some(TASK_ACTION_SCHEMA_VERSION.into()),
        action: RecordedTaskActionKind::Complete,
        note: None,
    }, Some(signal)).await
}

async fn record_task_action_with_calendar(
    state: AppState,
    task_id: String,
    request: RecordedTaskActionRequest,
    calendar: Option<&super::calendar_interaction::CompletionSignal>,
) -> Result<RecordedTaskActionResponse> {
    let _guard = state.inner().task_action_lock.lock().await;
    validate_schema_version(request.schema_version.as_deref())?;

    let effective_time = state.planning_now();
    let pool = state.inner().store.pool();
    let mut task = load_task(pool, &task_id).await?;
    if let Some(signal) = calendar {
        super::calendar_range::CalendarTimeRange::parse(&signal.observed_start, &signal.observed_end)
            .map_err(|_| AppError::bad_request_diagnostic("capture_interaction_invalid_window", "Calendar completion requires a valid observed window"))?;
        if task.payload.get("static_window").is_some_and(|window| !window.is_null()) || task.payload["provenance"]["source"]["source_kind"] == "google_calendar" {
            return Err(AppError::conflict_diagnostic("capture_interaction_not_dynamic", "Task became Static or captured before Calendar completion; gesture ignored"));
        }
    }
    let authority_source = authority_for_recorded_action(request.action);
    let mut diagnostics = Vec::new();
    let transition_applied = if matches!(request.action, RecordedTaskActionKind::Complete) {
        apply_completed_transition(&state, &mut task, authority_source, effective_time).await?;
        // A completed Task applies its effects to UniverseState; the effect
        // applies because the Task completed, so success_probability is ignored.
        diagnostics.extend(
            apply_completed_effects(
                &state,
                &task,
                authority_source,
                MVP_INSTANCE_MODE,
                effective_time,
            )
            .await?,
        );
        true
    } else if matches!(request.action, RecordedTaskActionKind::Skip) {
        apply_skipped_transition(&state, &mut task, effective_time).await?;
        true
    } else {
        false
    };

    let authority_source_wire = authority_source_wire(authority_source)?;
    let task_status = task_status_from_wire(&task.status)?;

    let mut payload = json!({
        "schema_version": TASK_ACTION_SCHEMA_VERSION,
        "action": recorded_action_wire(request.action),
        "decision": recorded_decision_wire(request.action),
        "task_status": task.status,
        "transition_applied": transition_applied,
    });
    if let Some(note) = &request.note {
        payload["note"] = json!(note);
    }

    if let Some(signal) = calendar {
        payload["source"] = json!({"source_kind":"google_calendar","source_id":signal.external_id});
        payload["observed_window"] = json!({"start":signal.observed_start,"end":signal.observed_end});
    }

    let log_id = append_task_decision(&state, &task, payload, Vec::new(), authority_source, effective_time).await?;

    if matches!(request.action, RecordedTaskActionKind::Snooze) {
        // TODO(O6-snooze-readiness): Snooze records a defer decision only;
        // snooze-aware readiness is intentionally deferred out of this slice.
        diagnostics.push(crate::api::user_action::ActionDiagnostic {
            code: "snooze_readiness_deferred".to_owned(),
            message: "snooze was recorded without changing readiness evaluation".to_owned(),
        });
    }

    Ok(RecordedTaskActionResponse {
        schema_version: TASK_ACTION_SCHEMA_VERSION.to_owned(),
        log_id,
        task_id,
        action: request.action,
        task_status,
        authority_source: authority_source_wire,
        transition_applied,
        note: request.note,
        diagnostics,
    })
}

/// The narrow inverse of a Calendar completion; no new public action vocabulary.
/// Re-check the most recent completion under the same lock as app actions.
pub async fn reopen_calendar_completion(
    state: &AppState,
    signal: &super::calendar_interaction::ReopenSignal,
) -> Result<bool> {
    let _guard = state.inner().task_action_lock.lock().await;
    let pool = state.inner().store.pool();
    let mut task = load_task(pool, &signal.task_id).await?;
    if task.status != "completed" || task.payload.get("static_window").is_some_and(|window| !window.is_null()) || task.payload["provenance"]["source"]["source_kind"] == "google_calendar" { return Ok(false); }
    let Some(completion) = super::calendar_interaction::latest_completion(pool, &task.id).await? else { return Ok(false); };
    if completion.log_id != signal.completion_log_id || !completion.is_from_calendar_event(&signal.external_id) { return Ok(false); }
    let now = state.planning_now();
    persist_task_transition(state, &mut task, "active", AuthoritySource::User, now).await?;
    let payload = json!({
        "schema_version":TASK_ACTION_SCHEMA_VERSION, "action":"reopen", "decision":"task_reopened",
        "task_status":"active", "transition_applied":true,
        "source":{"source_kind":"google_calendar","source_id":signal.external_id},
        "completion_log_id":signal.completion_log_id
    });
    append_task_decision(state, &task, payload, vec![signal.completion_log_id.clone()], AuthoritySource::User, now).await?;
    Ok(true)
}

/// Keep canonical decision recording shared by app actions and Calendar undo.
async fn append_task_decision(
    state: &AppState,
    task: &TaskForTransition,
    payload: serde_json::Value,
    extra_refs: Vec<String>,
    authority_source: AuthoritySource,
    effective_time: UbuTimestamp,
) -> Result<String> {
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let now = effective_time.to_string();
    let mut refs = vec![task.id.clone()];
    refs.extend(extra_refs);
    // The closed Log vocabulary uses decision_recorded for Task transitions.
    let envelope = state.envelope_for(
        [(UbuId::parse(&task.id)?, observed_version(task.version)?)].into_iter().collect(),
        authority_source, effective_time,
    )?;
    queries::append_log_entry(state.inner().store.pool(), &envelope, NewLogRecord {
        id:log_id.clone(), event_type:"decision_recorded".into(), object_refs:json!(refs), payload,
        provenance:json!({"created_at":now,"authority_source":authority_source_wire(authority_source)?}),
        created_at:now,
    }).await?;
    Ok(log_id)
}

pub async fn append_action(
    state: AppState,
    task_id: String,
    action: TaskActionKind,
    request: UserActionRequest,
) -> Result<LogEntryResponse> {
    let _guard = state.inner().task_action_lock.lock().await;
    if UbuId::parse(&task_id).is_ok() {
        if let Some(record) = queries::get_current_state(state.inner().store.pool(), &task_id).await? {
            let payload: serde_json::Value = serde_json::from_str(&record.payload_json)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            if record.object_type == ObjectType::Task.as_str() && payload.get("occurrence").is_some_and(|v| !v.is_null()) {
                return Err(AppError::bad_request_diagnostic("use_recorded_action", "Routine occurrences require POST /task/:id/action"));
            }
        }
    }
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let now = state.planning_now().to_string();
    let event_type = action_event_type(action);
    let status = TaskLifecycleStatus::for_action(action);

    let mut payload = json!({
        "action": format!("{action:?}").to_ascii_lowercase(),
        "task_status": format!("{status:?}").to_ascii_lowercase(),
    });
    if let Some(note) = &request.note {
        payload["note"] = json!(note);
    }

    // Legacy action endpoint records user intent; it does not read Task state.
    let envelope = state.envelope_for(
        Default::default(),
        AuthoritySource::User,
        UbuTimestamp::parse(&now)?,
    )?;
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        NewLogRecord {
            id: log_id.clone(),
            event_type,
            object_refs: json!([task_id]),
            payload,
            provenance: json!({
                "created_at": now,
                "authority_source": "user"
            }),
            created_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    drop(_guard);
    recalculation_service::recalculate(state).await?;

    Ok(LogEntryResponse {
        log_id,
        task_id,
        action,
        status,
        authority_source: "user".to_owned(),
        note: request.note,
    })
}

fn action_event_type(action: TaskActionKind) -> String {
    match action {
        TaskActionKind::Start => "task_started",
        TaskActionKind::Done => "task_done",
        TaskActionKind::Snooze => "task_snoozed",
        TaskActionKind::Reject => "task_rejected",
        TaskActionKind::Decompose => "task_decomposed",
    }
    .to_owned()
}

struct TaskForTransition {
    id: String,
    status: String,
    payload: serde_json::Value,
    version: i64,
    compartment_label: String,
    created_at: String,
}

async fn load_task(pool: &sqlx::SqlitePool, task_id: &str) -> Result<TaskForTransition> {
    let record = queries::get_current_state(pool, task_id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::NotFound(format!("task `{task_id}` not found")))?;

    if record.object_type != ObjectType::Task.as_str() {
        return Err(AppError::bad_request_diagnostic(
            "not_a_task",
            format!("object `{task_id}` is not a Task"),
        ));
    }

    let payload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Internal(format!("failed to deserialize task: {e}")))?;

    Ok(TaskForTransition {
        id: record.id,
        status: record.status,
        payload,
        version: record.version,
        compartment_label: record.compartment_label,
        created_at: record.created_at,
    })
}

fn observed_version(version: i64) -> Result<VersionRef> {
    Ok(VersionRef::Version(u64::try_from(version).map_err(
        |e| AppError::Internal(format!("invalid stored object version: {e}")),
    )?))
}

async fn apply_completed_transition(
    state: &AppState,
    task: &mut TaskForTransition,
    authority_source: AuthoritySource,
    effective_time: UbuTimestamp,
) -> Result<()> {
    if task.status != "active" && !(task.status == "failed" && task.payload.get("occurrence").is_some_and(|v| !v.is_null())) {
        return Err(AppError::bad_request_diagnostic(
            "invalid_task_state",
            "complete requires an active Task or a missed routine occurrence",
        ));
    }
    persist_task_transition(state, task, "completed", authority_source, effective_time).await
}

async fn persist_task_transition(
    state: &AppState,
    task: &mut TaskForTransition,
    status: &str,
    authority_source: AuthoritySource,
    effective_time: UbuTimestamp,
) -> Result<()> {
    let envelope = state.envelope_for(
        [(UbuId::parse(&task.id)?, observed_version(task.version)?)]
            .into_iter()
            .collect(),
        authority_source,
        effective_time,
    )?;
    let mut payload = task.payload.clone();
    payload["status"] = json!(status);
    let admitted = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: task.id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: task.version,
            status: status.to_owned(),
            compartment_label: task.compartment_label.clone(),
            payload: payload.clone(),
            created_at: task.created_at.clone(),
            updated_at: effective_time.to_string(),
        },
    )
    .await?;
    task.status = admitted.status;
    task.version = admitted.version;
    task.payload = payload;
    Ok(())
}

async fn apply_skipped_transition(state: &AppState, task: &mut TaskForTransition, now: UbuTimestamp) -> Result<()> {
    if !task.payload.get("occurrence").is_some_and(|v| !v.is_null()) {
        return Err(AppError::bad_request_diagnostic("not_a_routine_occurrence", "skip only applies to a routine occurrence"));
    }
    if !matches!(task.status.as_str(), "active" | "failed") {
        return Err(AppError::bad_request_diagnostic("invalid_task_state", "skip requires an active or missed routine occurrence"));
    }
    let envelope = state.envelope_for([(UbuId::parse(&task.id)?, observed_version(task.version)?)].into_iter().collect(), AuthoritySource::User, now)?;
    let mut payload = task.payload.clone();
    payload["status"] = json!("moot");
    payload["moot_reason_code"] = json!("user_declared_moot");
    let admitted = queries::admit_object(state.inner().store.pool(), &envelope, NewObjectRecord {
        id: task.id.clone(), object_type: ObjectType::Task.as_str().into(), version: task.version,
        status: "moot".into(), compartment_label: task.compartment_label.clone(), payload: payload.clone(),
        created_at: task.created_at.clone(), updated_at: now.to_string(),
    }).await?;
    task.status = admitted.status; task.version = admitted.version; task.payload = payload;
    Ok(())
}

/// Apply a completed Task's effects to the current `UniverseState` (§10.2).
///
/// The effect applies because the Task completed, so `success_probability` is
/// planning metadata only and never gates application here. Effects mutations
/// are validated against the instance mode (Wiring-B) and applied with the pure
/// `ubu-core` applicator (C9); the result is persisted as a new current version
/// through the store's `persist_universe_state` (ST7) under the completing
/// action's authority. No SQL is written against UniverseState. Mode or
/// application failures surface as diagnostics without partially persisting.
///
/// Only completed transitions reach this path; a Task that transitions to
/// `failed` applies nothing.
async fn apply_completed_effects(
    state: &AppState,
    task: &TaskForTransition,
    authority_source: AuthoritySource,
    mode: InstanceMode,
    effective_time: UbuTimestamp,
) -> Result<Vec<ActionDiagnostic>> {
    let pool = state.inner().store.pool();
    let Some(effects_value) = task.payload.get("effects") else {
        return Ok(Vec::new());
    };
    let effect: TaskEffect = serde_json::from_value(effects_value.clone())
        .map_err(|e| AppError::Internal(format!("failed to deserialize task effects: {e}")))?;

    // `success_probability` is intentionally ignored: completion, not the
    // predicted probability, is what makes the effect apply.
    if effect.mutations.is_empty() {
        return Ok(Vec::new());
    }

    // Mode validation: an intrinsic-affect mutation target under a mode that
    // does not model intrinsic affect makes the effect invalid (surfaced
    // distinctly); `user_mode` always permits.
    if let Err(error) = validate_mutations_for_mode(mode, &effect.mutations) {
        return Ok(vec![ActionDiagnostic {
            code: "task_effect_mode_invalid".to_owned(),
            message: error.to_string(),
        }]);
    }

    let current = planning_service::read_current_universe_state(pool).await?;
    let (current_state, current_version) = if let Some(current) = current {
        current
    } else {
        let seed = UniverseState::new(effective_time, "empty UniverseState seeded on Task completion");
        let now = effective_time.to_string();
        let mut payload = serde_json::to_value(&seed)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        payload["schema_version"] = json!("core/universe-state/0.1");
        payload["provenance"] = json!({"created_at":effective_time,"authority_source":authority_source});
        let envelope = state.envelope_for(
            [
                (seed.id.clone(), VersionRef::Absent),
                (UbuId::parse(&task.id)?, observed_version(task.version)?),
            ].into_iter().collect(),
            authority_source,
            effective_time,
        )?;
        queries::admit_object(pool, &envelope, NewObjectRecord {
            id: seed.id.to_string(),
            object_type: ObjectType::UniverseState.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: task.compartment_label.clone(),
            payload,
            created_at: now.clone(),
            updated_at: now,
        }).await?;
        planning_service::read_current_universe_state(pool).await?
            .ok_or_else(|| AppError::Internal("UniverseState missing after seed admission".into()))?
    };

    let next_state = match apply_universe_mutations(&current_state, &effect.mutations) {
        Ok(next_state) => next_state,
        Err(error) => {
            return Ok(vec![ActionDiagnostic {
                code: "task_effect_application_failed".to_owned(),
                message: error.to_string(),
            }]);
        }
    };

    // Reuse the version read with UniverseState; the effects also depend on the
    // just-completed Task version whose effects were applied.
    let envelope = state.envelope_for(
        [
            (current_state.id.clone(), observed_version(current_version)?),
            (UbuId::parse(&task.id)?, observed_version(task.version)?),
        ]
        .into_iter()
        .collect(),
        authority_source,
        effective_time,
    )?;
    queries::persist_universe_state(pool, &envelope, &next_state, authority_source)
        .await
        .map_err(AppError::from)?;

    Ok(Vec::new())
}

fn validate_schema_version(schema_version: Option<&str>) -> Result<()> {
    match schema_version {
        Some(TASK_ACTION_SCHEMA_VERSION) => Ok(()),
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

fn authority_for_recorded_action(action: RecordedTaskActionKind) -> AuthoritySource {
    match action {
        RecordedTaskActionKind::Start | RecordedTaskActionKind::Complete | RecordedTaskActionKind::Skip | RecordedTaskActionKind::Snooze => AuthoritySource::User,
        RecordedTaskActionKind::Override => AuthoritySource::UserOverride,
    }
}

fn authority_source_wire(authority_source: AuthoritySource) -> Result<String> {
    let serialized =
        serde_json::to_string(&authority_source).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(serialized.trim_matches('"').to_owned())
}

fn recorded_action_wire(action: RecordedTaskActionKind) -> &'static str {
    match action {
        RecordedTaskActionKind::Start => "start",
        RecordedTaskActionKind::Complete => "complete",
        RecordedTaskActionKind::Skip => "skip",
        RecordedTaskActionKind::Override => "override",
        RecordedTaskActionKind::Snooze => "snooze",
    }
}

fn recorded_decision_wire(action: RecordedTaskActionKind) -> &'static str {
    match action {
        RecordedTaskActionKind::Start => "task_started",
        RecordedTaskActionKind::Complete => "task_completed",
        RecordedTaskActionKind::Skip => "occurrence_skipped",
        RecordedTaskActionKind::Override => "recommendation_rejected",
        RecordedTaskActionKind::Snooze => "defer",
    }
}

fn task_status_from_wire(status: &str) -> Result<TaskLifecycleStatus> {
    match status {
        "active" => Ok(TaskLifecycleStatus::Active),
        "completed" => Ok(TaskLifecycleStatus::Completed),
        "failed" => Ok(TaskLifecycleStatus::Failed),
        "moot" => Ok(TaskLifecycleStatus::Moot),
        other => Err(AppError::Internal(format!(
            "stored Task has unsupported lifecycle status `{other}`"
        ))),
    }
}

#[cfg(test)]
mod effect_mode_tests {
    use super::*;
    use crate::config::ServerConfig;

    fn completed_task_with_effects(effects: serde_json::Value) -> TaskForTransition {
        TaskForTransition {
            id: UbuId::new(ObjectType::Task).to_string(),
            status: "completed".to_owned(),
            payload: json!({ "effects": effects }),
            version: 2,
            compartment_label: "test".into(),
            created_at: UbuTimestamp::now_utc().to_string(),
        }
    }

    fn intrinsic_affect_effect() -> serde_json::Value {
        json!({
            "mutations": [
                {
                    "operation": "increment_numeric",
                    "target": "numeric_values.affect.energy",
                    "payload": 1.0
                }
            ]
        })
    }

    #[tokio::test]
    async fn organization_mode_rejects_intrinsic_affect_effect() {
        let state = AppState::in_memory(ServerConfig::from_env())
            .await
            .expect("state");
        let task = completed_task_with_effects(intrinsic_affect_effect());
        let diagnostics = apply_completed_effects(
            &state,
            &task,
            AuthoritySource::User,
            InstanceMode::OrganizationMode,
            UbuTimestamp::now_utc(),
        )
        .await
        .expect("effects evaluated");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "task_effect_mode_invalid");
    }

    #[tokio::test]
    async fn user_mode_permits_intrinsic_affect_effect() {
        // A real admitted Task supplies the observed version for seeding and
        // applying its permitted intrinsic-affect mutation on a cold store.
        let state = AppState::in_memory(ServerConfig::from_env())
            .await
            .expect("state");
        let mut task = completed_task_with_effects(intrinsic_affect_effect());
        let now = state.planning_now();
        task.payload["id"] = json!(task.id);
        task.payload["title"] = json!("Synthetic completed Task");
        task.payload["status"] = json!("completed");
        task.payload["provenance"] = json!({"created_at":now,"authority_source":"user"});
        let envelope = state.envelope_for(
            [(UbuId::parse(&task.id).unwrap(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User, now,
        ).unwrap();
        let admitted = queries::admit_object(state.inner().store.pool(), &envelope, NewObjectRecord {
            id: task.id.clone(), object_type: "Task".into(), version: 1,
            status: task.status.clone(), compartment_label: task.compartment_label.clone(),
            payload: task.payload.clone(), created_at: now.to_string(), updated_at: now.to_string(),
        }).await.unwrap();
        task.version = admitted.version;
        let diagnostics = apply_completed_effects(
            &state,
            &task,
            AuthoritySource::User,
            InstanceMode::UserMode,
            UbuTimestamp::now_utc(),
        )
        .await
        .expect("effects evaluated");
        assert!(diagnostics.is_empty());
        let (universe, _) = planning_service::read_current_universe_state(state.inner().store.pool())
            .await.unwrap().unwrap();
        assert_eq!(universe.numeric_values["affect.energy"], 1.0);
    }
}
