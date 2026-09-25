//! Synthetic HTTP integration tests. The only Calendar client is a recorder.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_apply::last_applied_events,
        calendar_client::{CalendarApi, RecordedCalendarCall, RecordingCalendarApi},
        calendar_projection::{external_id, DesiredEvent},
    },
    state::AppState,
};

const NOW: &str = "2026-09-25T08:00:00Z";
const END: &str = "2026-09-25T20:00:00Z";
const RECONCILE: &str = "/projection/calendar/reconcile";
const RECONCILE_SCHEMA: &str = "ubu.orchestrator.calendar_reconciliation.v1";

async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, value) = request(state, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {value}");
    value
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, preview: &Value) -> Value {
    ok(state, "POST", "/projection/calendar/approve", json!({
        "schema_version":"ubu.orchestrator.calendar_projection_approval.v1",
        "preview_id":preview["preview_id"],"authority_source":"automation_worker","export_mode":"mock"
    })).await
}
async fn reconcile(state: &AppState) -> Value {
    ok(
        state,
        "POST",
        RECONCILE,
        json!({"schema_version":RECONCILE_SCHEMA,"export_mode":"mock"}),
    )
    .await
}
fn repair_path(reconciliation: &Value) -> String {
    format!(
        "{RECONCILE}/{}/repair",
        reconciliation["reconciliation_id"].as_str().unwrap()
    )
}
async fn repair(state: &AppState, reconciliation: &Value) -> Value {
    ok(state, "POST", &repair_path(reconciliation), Value::Null).await
}
async fn generate(state: &AppState) {
    let result = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":END}}),
    )
    .await;
    assert!(result["plan"].is_object(), "{result}");
}
async fn setup() -> (AppState, Arc<RecordingCalendarApi>, Value) {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder.clone());
    for (title, hour) in [("Breakfast", "09"), ("Standup", "10")] {
        ok(&state,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1", "title":title,
            "static_window":{"start":format!("2026-09-25T{hour}:00:00Z"),"end":format!("2026-09-25T{hour}:30:00Z")},
            "category_tag":"personal","tags":["personal"],"occupies_capacity":true})).await;
    }
    generate(&state).await;
    let desired = preview(&state).await;
    assert_eq!(desired["events"].as_array().unwrap().len(), 2);
    (state, recorder, desired)
}
async fn setup_applied() -> (AppState, Arc<RecordingCalendarApi>, Value) {
    let (state, recorder, desired) = setup().await;
    assert_eq!(approve(&state, &desired).await["status"], "applied");
    recorder.clear_recorded_calls();
    (state, recorder, desired)
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn canonical(state: &AppState) -> Value {
    let mut snapshot = json!({});
    for table in ["objects", "logs", "plans", "calendars"] {
        let values: Vec<(String, String)> =
            sqlx::query_as(&format!("SELECT id,payload_json FROM {table} ORDER BY id"))
                .fetch_all(state.inner().store.pool())
                .await
                .unwrap();
        snapshot[table] = serde_json::to_value(values).unwrap();
    }
    let versions: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT id,version,status FROM objects ORDER BY id")
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    snapshot["object_versions"] = serde_json::to_value(versions).unwrap();
    snapshot
}
async fn applied(state: &AppState) -> Vec<DesiredEvent> {
    last_applied_events(state.inner().store.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn unchanged_recording_calendar_matches_without_changing_belief_or_canonical_state() {
    let (state, recorder, _) = setup_applied().await;
    let before = canonical(&state).await;
    let belief = applied(&state).await;
    let result = reconcile(&state).await;
    assert_eq!(result["status"], "matched");
    assert_eq!(result["conflicts"], json!([]));
    assert_eq!(result["diagnostics"], json!([]));
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(applied(&state).await, belief);
    assert_eq!(canonical(&state).await, before);
    assert_eq!(count(&state, "projection_results").await, 1);
    let raw: String =
        sqlx::query_scalar("SELECT payload_json FROM projection_reconciliations WHERE id=?")
            .bind(result["reconciliation_id"].as_str().unwrap())
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    let stored: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["schema_version"], RECONCILE_SCHEMA);
    assert_eq!(
        stored["applied_events"],
        serde_json::to_value(&belief).unwrap()
    );
    assert_eq!(
        stored["observed_events"],
        serde_json::to_value(recorder.events()).unwrap()
    );
    assert_eq!(stored["repaired"], false);
    recorder.clear_recorded_calls();
    for (body, expected) in [
        (json!({"export_mode":"mock"}), StatusCode::BAD_REQUEST),
        (
            json!({"schema_version":"unknown","export_mode":"mock"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"schema_version":RECONCILE_SCHEMA,"export_mode":"mock","observed_events":[]}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_eq!(request(&state, "POST", RECONCILE, body).await.0, expected);
    }
    assert!(recorder.recorded_calls().is_empty());
}

#[tokio::test]
async fn apply_wipe_reconcile_repair_preview_recreates_every_event() {
    let (state, recorder, desired) = setup_applied().await;
    let belief = applied(&state).await;
    for event in recorder.events() {
        recorder.delete_event(&event.external_id).await.unwrap();
    }
    recorder.clear_recorded_calls();
    assert!(recorder.events().is_empty());
    assert_eq!(preview(&state).await["operations"], json!([]));
    let before = canonical(&state).await;
    let result = reconcile(&state).await;
    assert_eq!(result["status"], "drifted");
    let conflicts = result["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), belief.len());
    for (conflict, event) in conflicts.iter().zip(&belief) {
        assert_eq!(
            conflict,
            &json!({"external_id":event.external_id,"conflict_type":"missing","summary":event.summary,
            "message":"UbU applied this event and the calendar no longer has it"})
        );
    }
    println!("P1B31_WIPED_CONFLICTS {}", result["conflicts"]);
    assert_eq!(applied(&state).await, belief); // Observing alone repairs nothing.
    let repaired = repair(&state, &result).await;
    assert_eq!(repaired["dropped_events"], 2);
    assert_eq!(repaired["updated_events"], 0);
    assert_eq!(repaired["applied_event_count"], 0);
    assert_eq!(repaired["remaining_conflicts"], json!([]));
    assert!(applied(&state).await.is_empty());
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(canonical(&state).await, before);
    let fresh = preview(&state).await;
    assert_eq!(fresh["operations"], desired["operations"]);
    assert!(fresh["operations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|op| op["kind"] == "create"));
    println!("P1B31_WIPED_OPERATIONS {}", fresh["operations"]);
    assert_eq!(approve(&state, &fresh).await["status"], "applied");
    assert_eq!(recorder.events(), belief);
    assert_eq!(reconcile(&state).await["status"], "matched");
}

#[tokio::test]
async fn externally_dragged_event_becomes_exactly_one_ordinary_update() {
    let (state, recorder, _) = setup_applied().await;
    let original = recorder
        .events()
        .into_iter()
        .find(|event| event.summary == "Breakfast")
        .unwrap();
    let mut moved = original.clone();
    moved.start_at = "2026-09-25T09:10:00Z".into();
    moved.end_at = "2026-09-25T09:40:00Z".into();
    recorder.patch_event(&moved).await.unwrap();
    recorder.clear_recorded_calls();
    assert_eq!(preview(&state).await["operations"], json!([]));
    let result = reconcile(&state).await;
    assert_eq!(result["status"], "drifted");
    assert_eq!(result["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(result["conflicts"][0]["conflict_type"], "drifted");
    let repaired = repair(&state, &result).await;
    assert_eq!(repaired["dropped_events"], 0);
    assert_eq!(repaired["updated_events"], 1);
    assert!(applied(&state).await.contains(&moved));
    let fresh = preview(&state).await;
    assert_eq!(
        fresh["operations"],
        json!([{"kind":"update","event":original}])
    );
    println!("P1B31_EDITED_OPERATIONS {}", fresh["operations"]);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
}

#[tokio::test]
async fn foreign_event_is_reported_and_survives_without_adoption_or_operations() {
    let (state, _, _) = setup_applied().await;
    let belief = applied(&state).await;
    let mut foreign = belief[0].clone();
    foreign.external_id = "00000000000000000000000000dddddd".into();
    foreign.task_id = format!("task_{}", foreign.external_id);
    foreign.summary = "Dentist".into();
    let recorder = Arc::new(RecordingCalendarApi::with_events(
        belief.iter().cloned().chain([foreign.clone()]),
    ));
    let state = state.with_calendar_api(recorder.clone());
    let before = canonical(&state).await;
    let result = reconcile(&state).await;
    assert_eq!(result["status"], "observed");
    assert_eq!(
        result["conflicts"],
        json!([{"external_id":foreign.external_id,"conflict_type":"foreign","summary":"Dentist",
        "message":"this event was not created by UbU and will not be touched"}])
    );
    let repaired = repair(&state, &result).await;
    assert_eq!(repaired["applied_event_count"], 2);
    assert_eq!(repaired["dropped_events"], 0);
    assert_eq!(repaired["updated_events"], 0);
    assert_eq!(repaired["remaining_conflicts"], result["conflicts"]);
    assert_eq!(applied(&state).await, belief);
    assert!(recorder.events().contains(&foreign));
    assert_eq!(recorder.events().len(), 3);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(canonical(&state).await, before);
    let operations = preview(&state).await["operations"].clone();
    assert_eq!(operations, json!([]));
    assert!(!operations.to_string().contains(&foreign.external_id));
    println!(
        "P1B31_FOREIGN repaired applied-set size: {}; observed size: {}; operations: {}",
        repaired["applied_event_count"],
        recorder.events().len(),
        operations
    );
}

#[tokio::test]
async fn unrecorded_id_of_an_active_unscheduled_task_is_not_foreign() {
    let (state, recorder, _) = setup().await; // No apply history, as after a reset.
    let capture = ok(&state,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Unscheduled synthetic Task"})).await;
    let task_id = capture["task_id"].as_str().unwrap();
    let event = DesiredEvent {
        external_id: external_id(task_id).unwrap(),
        task_id: task_id.into(),
        summary: "Unscheduled synthetic Task".into(),
        start_at: "2026-09-25T11:00:00Z".into(),
        end_at: "2026-09-25T11:30:00Z".into(),
        color_id: None,
        transparent: false,
        reminders_minutes: vec![],
    };
    recorder.insert_event(&event).await.unwrap();
    recorder.clear_recorded_calls();
    let result = reconcile(&state).await;
    assert_eq!(result["status"], "observed");
    assert_eq!(
        result["conflicts"],
        json!([{"external_id":event.external_id,"conflict_type":"unrecorded","summary":event.summary,
        "message":"this event matches a known Task but UbU has no applied record; it will not be adopted"}])
    );
    println!("P1B31_UNRECORDED {}", result["conflicts"]);
    let repaired = repair(&state, &result).await;
    assert_eq!(repaired["applied_event_count"], 0);
    assert_eq!(repaired["remaining_conflicts"], result["conflicts"]);
    assert!(applied(&state).await.is_empty());
    assert_eq!(recorder.events(), vec![event]);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    ok(
        &state,
        "POST",
        &format!("/task/{task_id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    // The helper includes active Tasks only; this event still gains no ownership.
    assert_eq!(
        reconcile(&state).await["conflicts"][0]["conflict_type"],
        "foreign"
    );
}

#[tokio::test]
async fn repair_is_atomic_and_second_or_concurrent_repair_is_rejected() {
    let (state, recorder, _) = setup_applied().await;
    for event in recorder.events() {
        recorder.delete_event(&event.external_id).await.unwrap();
    }
    recorder.clear_recorded_calls();
    let result = reconcile(&state).await;
    let before = applied(&state).await;
    let count_before = count(&state, "projection_results").await;
    let pool = state.inner().store.pool();
    sqlx::query("CREATE TRIGGER synthetic_repair_failure BEFORE UPDATE ON projection_reconciliations WHEN NEW.status='repaired' BEGIN SELECT RAISE(ABORT, 'synthetic repair failure'); END")
        .execute(pool).await.unwrap();
    assert_eq!(
        request(&state, "POST", &repair_path(&result), Value::Null)
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(count(&state, "projection_results").await, count_before);
    assert_eq!(applied(&state).await, before);
    sqlx::query("DROP TRIGGER synthetic_repair_failure")
        .execute(pool)
        .await
        .unwrap();
    let path = repair_path(&result);
    let (one, two) = tokio::join!(
        request(&state, "POST", &path, Value::Null),
        request(&state, "POST", &path, Value::Null)
    );
    let statuses = [one.0, two.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let (status, body) = request(&state, "POST", &path, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["diagnostics"][0]["code"],
        "calendar_reconciliation_already_repaired"
    );
    assert_eq!(count(&state, "projection_results").await, count_before + 1);
    assert!(applied(&state).await.is_empty());
    let (status, raw): (String, String) =
        sqlx::query_as("SELECT status,payload_json FROM projection_reconciliations WHERE id=?")
            .bind(result["reconciliation_id"].as_str().unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "repaired");
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap()["repaired"],
        true
    );
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(
        request(
            &state,
            "POST",
            &format!("{RECONCILE}/synthetic-absent/repair"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO projection_reconciliations (id,preview_id,result_id,status,payload_json,created_at) VALUES ('synthetic-github','','','matched',?,?)")
        .bind(json!({"schema_version":"ubu.orchestrator.projection_reconciliation.v1","observed_labels":[]}).to_string())
        .bind(NOW).execute(pool).await.unwrap();
    assert_eq!(
        request(
            &state,
            "POST",
            &format!("{RECONCILE}/synthetic-github/repair"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(count(&state, "projection_results").await, count_before + 1);
}

#[tokio::test]
async fn live_without_configuration_or_enablement_is_refused_before_any_client_call() {
    let base = ServerConfig::from_env();
    for (config, status, code) in [
        (
            base.clone(),
            StatusCode::SERVICE_UNAVAILABLE,
            "calendar_live_export_unconfigured",
        ),
        (
            base.with_google_credentials_path("/synthetic-not-present/oauth.json")
                .with_google_token_cache_path("/synthetic-not-present/cache.json"),
            StatusCode::FORBIDDEN,
            "calendar_live_export_not_enabled",
        ),
    ] {
        let recorder = Arc::new(RecordingCalendarApi::new());
        let state = AppState::in_memory(config)
            .await
            .unwrap()
            .with_calendar_api(recorder.clone());
        let (actual, body) = request(
            &state,
            "POST",
            RECONCILE,
            json!({"schema_version":RECONCILE_SCHEMA,"export_mode":"live"}),
        )
        .await;
        assert_eq!(actual, status);
        assert_eq!(body["diagnostics"][0]["code"], code);
        assert!(recorder.recorded_calls().is_empty());
        for table in ["projection_reconciliations", "projection_results", "logs"] {
            assert_eq!(count(&state, table).await, 0);
        }
    }
}
