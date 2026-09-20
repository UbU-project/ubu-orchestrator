use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, MutationEnvelope, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::api::bootstrap::BOOTSTRAP_SCHEMA_VERSION;
use ubu_orchestrator::api::user_action::TASK_ACTION_SCHEMA_VERSION;
use ubu_orchestrator::config::{GithubIngestMode, ProjectionExportMode, ServerConfig};
use ubu_orchestrator::device_registration::new_registration;
use ubu_orchestrator::state::AppState;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::models::recorded_mutation::RecordedMutation;
use ubu_store::{queries, StoreError};

async fn state() -> AppState {
    AppState::in_memory_with_registration(
        ServerConfig::from_env()
            .with_github_ingest_mode(GithubIngestMode::Mock)
            .with_github_projection_export_mode(ProjectionExportMode::Mock),
        new_registration(),
    )
    .await
    .unwrap()
}

async fn post(state: &AppState, uri: &str, body: Value) -> Value {
    let response = ubu_orchestrator::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

async fn ledger(state: &AppState) -> Vec<RecordedMutation> {
    sqlx::query_as("SELECT * FROM mutation_envelopes ORDER BY recorded_at, idempotency_key")
        .fetch_all(state.inner().store.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn bootstrap_complete_and_override_all_carry_the_registered_device_and_identity() {
    let state = state().await;
    let registration = &state.inner().device_registration;
    let seeded = post(
        &state,
        "/bootstrap/seed",
        json!({
            "schema_version": BOOTSTRAP_SCHEMA_VERSION,
            "selected_repo": {"owner": "UbU-project", "repo": "ubu-orchestrator"},
            "answers": {"primary_objective": "Verify envelopes", "work_style": "focused",
                "planning_horizon_days": 7, "attention_preference": "deep_work"}
        }),
    )
    .await;
    let task_id = seeded["imported_tasks"]["candidates"][0]["task_id"]
        .as_str()
        .unwrap();
    let universe_id = seeded["universe_state_id"].as_str().unwrap();
    let bootstrap_ledger = ledger(&state).await;
    assert_eq!(bootstrap_ledger.len(), 7);
    let imported_time = UbuTimestamp::parse("2026-06-10T14:30:00Z").unwrap();
    for row in &bootstrap_ledger {
        let envelope: MutationEnvelope = serde_json::from_str(&row.envelope_json).unwrap();
        if row.result_object_id == task_id || row.result_object_id.starts_with("xref_") {
            assert_eq!(envelope.authority_source, AuthoritySource::System);
            assert_eq!(envelope.effective_time, imported_time);
        } else {
            assert_eq!(envelope.authority_source, AuthoritySource::User);
        }
        if row.result_object_id.starts_with("xref_") {
            assert!(envelope.observed_versions.is_empty());
        } else {
            assert_eq!(
                envelope
                    .observed_versions
                    .get(&UbuId::parse(&row.result_object_id).unwrap()),
                Some(&VersionRef::Absent)
            );
        }
    }

    // Give the imported Task an effect, so completion also exercises the versioned
    // UniverseState write. Even this fixture update uses the registered issuer.
    let current = queries::get_current_state(state.inner().store.pool(), task_id)
        .await
        .unwrap()
        .unwrap();
    let effective_time = UbuTimestamp::now_utc();
    let envelope = state
        .envelope_for(
            [(
                UbuId::parse(task_id).unwrap(),
                VersionRef::Version(current.version as u64),
            )]
            .into_iter()
            .collect(),
            AuthoritySource::User,
            effective_time,
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&current.payload_json).unwrap();
    payload["effects"] = json!({"mutations": [{"operation": "increment_numeric", "target": "numeric_values.test.completions", "payload": 1.0}]});
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: current.id,
            object_type: current.object_type,
            version: current.version,
            status: current.status,
            compartment_label: current.compartment_label,
            payload,
            created_at: current.created_at,
            updated_at: effective_time.to_string(),
        },
    )
    .await
    .unwrap();

    let override_action = post(&state, &format!("/task/{task_id}/action"), json!({
        "schema_version": TASK_ACTION_SCHEMA_VERSION, "action": "override", "note": "operator correction"
    })).await;
    let complete_action = post(
        &state,
        &format!("/task/{task_id}/action"),
        json!({
            "schema_version": TASK_ACTION_SCHEMA_VERSION, "action": "complete"
        }),
    )
    .await;
    assert_eq!(complete_action["task_status"], "completed");
    let rows = ledger(&state).await;
    assert_eq!(rows.len(), 12);
    let task_id = UbuId::parse(task_id).unwrap();
    let universe_id = UbuId::parse(universe_id).unwrap();
    let mut completion_time = None;
    let mut effects_time = None;
    let mut log_time = None;
    for row in rows {
        let envelope: MutationEnvelope = serde_json::from_str(&row.envelope_json).unwrap();
        envelope.validate().unwrap();
        assert_eq!(envelope.origin_device_id, registration.device_id);
        assert_eq!(
            envelope.actor_identity_id,
            registration.registered_identity_id
        );
        assert_eq!(envelope.created_time, envelope.recorded_time);
        assert_eq!(row.recorded_at, envelope.recorded_time.to_string());
        if row.result_object_id == task_id.as_str() && row.result_version == 3 {
            assert_eq!(
                envelope.observed_versions.get(&task_id),
                Some(&VersionRef::Version(2))
            );
            completion_time = Some(envelope.effective_time);
        }
        if row.result_object_id == universe_id.as_str() && row.result_version == 2 {
            assert_eq!(
                envelope.observed_versions.get(&universe_id),
                Some(&VersionRef::Version(1))
            );
            assert_eq!(
                envelope.observed_versions.get(&task_id),
                Some(&VersionRef::Version(3))
            );
            effects_time = Some(envelope.effective_time);
        }
        if row.result_object_id == override_action["log_id"].as_str().unwrap() {
            assert_eq!(envelope.authority_source, AuthoritySource::UserOverride);
            assert_eq!(
                envelope.observed_versions.get(&task_id),
                Some(&VersionRef::Version(2))
            );
        }
        if row.result_object_id == complete_action["log_id"].as_str().unwrap() {
            assert_eq!(envelope.authority_source, AuthoritySource::User);
            assert_eq!(
                envelope.observed_versions.get(&task_id),
                Some(&VersionRef::Version(3))
            );
            log_time = Some(envelope.effective_time);
        }
    }
    assert!(completion_time.is_some());
    assert_eq!(completion_time, effects_time);
    assert_eq!(completion_time, log_time);
    let universe = queries::get_current_state(state.inner().store.pool(), universe_id.as_str())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&universe.payload_json).unwrap()["numeric_values"]
            ["test.completions"],
        1.0
    );
}

#[tokio::test]
async fn issued_create_and_update_preconditions_are_real_and_replay_does_not_bump_twice() {
    let state = state().await;
    let id = UbuId::new(ObjectType::Task);
    let now = UbuTimestamp::now_utc();
    let mut record = NewObjectRecord {
        id: id.to_string(),
        object_type: "Task".into(),
        version: 1,
        status: "active".into(),
        compartment_label: "test".into(),
        payload: json!({"id": id, "title": "Original", "status": "active", "provenance": {
            "created_at": now, "authority_source": "user"}}),
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    let create = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    let first = queries::admit_object(state.inner().store.pool(), &create, record.clone())
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    let duplicate = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    assert!(matches!(
        queries::admit_object(state.inner().store.pool(), &duplicate, record.clone()).await,
        Err(StoreError::PreconditionFailed { .. })
    ));
    assert_eq!(ledger(&state).await.len(), 1);
    let update = state
        .envelope_for(
            [(id.clone(), VersionRef::Version(first.version as u64))]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    record.payload["title"] = json!("Updated");
    let updated = queries::admit_object(state.inner().store.pool(), &update, record.clone())
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    let before = ledger(&state).await;
    let stale = state
        .envelope_for(
            [(id.clone(), VersionRef::Version(1))].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    assert!(matches!(
        queries::admit_object(state.inner().store.pool(), &stale, record.clone()).await,
        Err(StoreError::PreconditionFailed { .. })
    ));
    assert_eq!(
        queries::admit_object(state.inner().store.pool(), &update, record)
            .await
            .unwrap(),
        updated
    );
    assert_eq!(
        queries::get_current_state(state.inner().store.pool(), id.as_str())
            .await
            .unwrap(),
        Some(updated)
    );
    assert_eq!(ledger(&state).await, before);
}
