//! Synthetic end-to-end capture evidence; every Calendar operation uses a recorder.
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
        calendar_projection::DesiredEvent,
    },
    state::AppState,
};
const NOW: &str = "2026-09-25T08:00:00Z";
const ORIGIN: &str = "5n0q8c9h7g4k2m1p3r6t8v0a2c";
fn event() -> DesiredEvent {
    DesiredEvent {
        external_id: ORIGIN.into(),
        task_id: format!("task_{ORIGIN}"),
        summary: "Dentist".into(),
        start_at: "2026-09-25T09:00:00Z".into(),
        end_at: "2026-09-25T09:30:00Z".into(),
        color_id: Some("3".into()),
        transparent: false,
        reminders_minutes: vec![],
    }
}
fn companion() -> DesiredEvent {
    let mut other = event();
    other.external_id = "bbbbb".into();
    other.task_id = "task_bbbbb".into();
    other.summary = "Synthetic commitment".into();
    other.start_at = "2026-09-25T12:00:00Z".into();
    other.end_at = "2026-09-25T12:30:00Z".into();
    other
}
async fn setup_with(recorder: Arc<RecordingCalendarApi>, config: ServerConfig) -> AppState {
    AppState::in_memory(config)
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder)
}
async fn setup(events: Vec<DesiredEvent>) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::with_events(events));
    (
        setup_with(recorder.clone(), ServerConfig::from_env()).await,
        recorder,
    )
}
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
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}
async fn capture(state: &AppState) -> Value {
    ok(
        state,
        "POST",
        "/projection/calendar/capture",
        json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"}),
    )
    .await
}
async fn generate(state: &AppState) {
    let result = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":"2026-09-25T20:00:00Z"}}),
    )
    .await;
    assert!(result["plan"].is_object(), "{result}");
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, preview: &Value) -> Value {
    ok(state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await
}
async fn reconcile(state: &AppState) -> Value {
    ok(state,"POST","/projection/calendar/reconcile",json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"})).await
}
async fn tasks(state: &AppState) -> Vec<Value> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task' ORDER BY id")
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    rows.into_iter()
        .map(|raw| serde_json::from_str(&raw).unwrap())
        .collect()
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn edit(state: &AppState, task: &Value, start: &str, end: &str) {
    ok(state,"PATCH",&format!("/task/{}",task["id"].as_str().unwrap()),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"static_window":{"start":start,"end":end}})).await;
}

#[tokio::test]
async fn foreign_event_admits_a_static_task_with_category_and_source_provenance() {
    let (state, recorder) = setup(vec![event()]).await;
    for schema in [Value::Null, json!("unsupported")] {
        let (status, _) = request(
            &state,
            "POST",
            "/projection/calendar/capture",
            json!({"schema_version":schema,"export_mode":"mock"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert!(recorder.recorded_calls().is_empty());
    let response = capture(&state).await;
    assert_eq!(
        (
            response["captured"].as_u64(),
            response["updated"].as_u64(),
            response["skipped"].as_u64()
        ),
        (Some(1), Some(0), Some(0))
    );
    assert_eq!(response["diagnostics"], json!([]));
    let tasks = tasks(&state).await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task["title"], "Dentist");
    assert_eq!(task["status"], "active");
    assert_eq!(
        task["static_window"],
        json!({"start":event().start_at,"end":event().end_at})
    );
    assert_eq!(task["category_tag"], "personal");
    assert_eq!(task["occupies_capacity"], true);
    assert_eq!(
        task["provenance"],
        json!({"created_at":NOW,"authority_source":"user","source":{"source_kind":"google_calendar","source_id":ORIGIN}})
    );
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(count(&state, "mutation_envelopes").await, 1);
    println!(
        "EVIDENCE[test1_task]={}",
        serde_json::to_string(task).unwrap()
    );
}

#[tokio::test]
async fn capture_generate_preview_and_apply_use_the_existing_origin_without_operations() {
    let mut seed = event();
    seed.start_at = "2026-09-25T11:00:00+02:00".into();
    seed.end_at = "2026-09-25T11:30:00+02:00".into();
    seed.reminders_minutes = vec![10];
    let (state, recorder) = setup(vec![seed.clone()]).await;
    capture(&state).await;
    generate(&state).await;
    let preview = preview(&state).await;
    assert_eq!(preview["events"].as_array().unwrap().len(), 1);
    assert_eq!(preview["events"][0]["external_id"], ORIGIN);
    assert_eq!(preview["operations"], json!([]));
    println!("EVIDENCE[test2_operations]={}", preview["operations"]);
    let applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].external_id, ORIGIN);
    assert_eq!(applied[0].task_id, tasks(&state).await[0]["id"]);
    assert_eq!(approve(&state, &preview).await["status"], "applied");
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(recorder.events(), vec![seed]);
    assert_eq!(reconcile(&state).await["conflicts"], json!([]));
}

#[tokio::test]
async fn rescheduling_updates_origin_and_retiring_task_never_deletes_meeting() {
    let (state, recorder) = setup(vec![event(), companion()]).await;
    capture(&state).await;
    let task = tasks(&state).await.remove(0);
    let (status,body) = request(&state,"PATCH",&format!("/task/{}",task["id"].as_str().unwrap()),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"static_window":null})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["diagnostics"][0]["code"], "capture_static_required");
    edit(
        &state,
        &task,
        "2026-09-25T10:00:00Z",
        "2026-09-25T10:30:00Z",
    )
    .await;
    generate(&state).await;
    let changed = preview(&state).await;
    let operations = changed["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["kind"], "update");
    assert_eq!(operations[0]["event"]["external_id"], ORIGIN);
    println!("EVIDENCE[test3_operation]={}", operations[0]);
    approve(&state, &changed).await;
    assert!(
        matches!(&recorder.recorded_calls()[1],RecordedCalendarCall::PatchEvent { event } if event.external_id == ORIGIN)
    );
    ok(
        &state,
        "POST",
        &format!("/task/{}/action", task["id"].as_str().unwrap()),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    generate(&state).await;
    let retired = preview(&state).await;
    assert!(retired["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["external_id"] != ORIGIN));
    assert_eq!(retired["operations"], json!([]));
    approve(&state, &retired).await;
    assert_eq!(recorder.events().len(), 2);
    assert!(recorder
        .events()
        .iter()
        .any(|event| event.external_id == ORIGIN));
    assert!(!recorder
        .recorded_calls()
        .iter()
        .any(|call| matches!(call, RecordedCalendarCall::DeleteEvent { .. })));
}

#[tokio::test]
async fn repeat_capture_reuses_one_source_and_performs_no_second_admission() {
    let (state, recorder) = setup(vec![event()]).await;
    capture(&state).await;
    let before = tasks(&state).await;
    let admissions = count(&state, "mutation_envelopes").await;
    let results = count(&state, "projection_results").await;
    let second = capture(&state).await;
    assert_eq!(second["captured"], 0);
    assert_eq!(second["updated"], 1);
    assert_eq!(second["skipped"], 0);
    assert_eq!(tasks(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions);
    assert_eq!(count(&state, "projection_results").await, results);
    let sources:i64=sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE json_extract(payload_json,'$.provenance.source.source_kind')='google_calendar' AND json_extract(payload_json,'$.provenance.source.source_id')=?").bind(ORIGIN).fetch_one(state.inner().store.pool()).await.unwrap();
    assert_eq!(sources, 1);
    let version: i64 = sqlx::query_scalar("SELECT version FROM objects WHERE object_type='Task'")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(version, 1);
    let mut moved = event();
    moved.summary = "Phone edit deferred".into();
    recorder.patch_event(&moved).await.unwrap();
    let drift = capture(&state).await;
    assert_eq!(drift["captured"], 0);
    assert_eq!(drift["updated"], 0);
    assert_eq!(drift["skipped"], 1);
    assert_eq!(tasks(&state).await, before);
    // Simulate bookkeeping loss after admission: provenance recovers the same Task.
    sqlx::query("DELETE FROM projection_results")
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let recovered = capture(&state).await;
    assert_eq!(recovered["captured"], 0);
    assert_eq!(recovered["updated"], 1);
    let after = tasks(&state).await;
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["id"], before[0]["id"]);
    assert_eq!(after[0]["title"], "Phone edit deferred");
}

#[tokio::test]
async fn an_event_ubu_projected_is_never_captured_as_a_new_task() {
    let (state, recorder) = setup(vec![]).await;
    ok(&state,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Synthetic work","static_window":{"start":event().start_at,"end":event().end_at}})).await;
    generate(&state).await;
    let initial = preview(&state).await;
    approve(&state, &initial).await;
    let before = tasks(&state).await;
    let admissions = count(&state, "mutation_envelopes").await;
    let results = count(&state, "projection_results").await;
    recorder.clear_recorded_calls();
    let response = capture(&state).await;
    assert_eq!(response["captured"], 0);
    assert_eq!(response["updated"], 0);
    assert_eq!(response["skipped"], 1);
    assert_eq!(tasks(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions);
    assert_eq!(count(&state, "projection_results").await, results);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert!(before[0]["provenance"].get("source").is_none());
    println!("EVIDENCE[test5_response]={response}");
    // Known active Task evidence also excludes an unrecorded ordinary event.
    sqlx::query("DELETE FROM projection_results")
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let unrecorded = capture(&state).await;
    assert_eq!(unrecorded["captured"], 0);
    assert_eq!(unrecorded["skipped"], 1);
    assert_eq!(tasks(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions);
}

#[tokio::test]
async fn transparent_foreign_event_does_not_occupy_capacity() {
    let mut free = event();
    free.transparent = true;
    free.color_id = Some("99".into());
    let (state, _) = setup(vec![free, companion()]).await;
    let response = capture(&state).await;
    assert_eq!(response["diagnostics"], json!([]));
    let task = tasks(&state)
        .await
        .into_iter()
        .find(|task| task["provenance"]["source"]["source_id"] == ORIGIN)
        .unwrap();
    assert_eq!(task["occupies_capacity"], false);
    assert!(task.get("category_tag").is_none());
    generate(&state).await;
    let plan = preview(&state).await;
    assert_eq!(plan["events"][0]["transparent"], true);
    assert_eq!(plan["operations"], json!([]));
}

#[tokio::test]
async fn ambiguous_colour_and_all_day_skips_are_explicit() {
    let palette = std::env::temp_dir().join(format!(
        "p1b-32-palette-{}.json",
        ubu_core::UbuId::new(ubu_core::ObjectType::Snapshot)
    ));
    std::fs::write(&palette, r#"{"second_personal":"3"}"#).unwrap();
    let wire = json!({"items":[{"id":ORIGIN,"summary":"Dentist","start":{"dateTime":event().start_at},"end":{"dateTime":event().end_at},"colorId":"3","reminders":{"useDefault":true}},{"id":"aaaaa","summary":"Synthetic all day","start":{"date":"2026-09-25"},"end":{"date":"2026-09-26"}}]});
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(&wire));
    let state = setup_with(
        recorder,
        ServerConfig::from_env().with_category_palette_path(&palette),
    )
    .await;
    std::fs::remove_file(palette).unwrap();
    let response = capture(&state).await;
    assert_eq!(response["captured"], 1);
    assert_eq!(response["skipped"], 1);
    let codes: Vec<_> = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["capture_all_day_unsupported", "capture_colour_ambiguous"]
    );
    assert!(tasks(&state).await[0].get("category_tag").is_none());
    println!("EVIDENCE[test7_diagnostics]={}", response["diagnostics"]);
    generate(&state).await;
    assert_eq!(preview(&state).await["operations"], json!([]));
}

#[tokio::test]
async fn vanished_source_is_reported_by_reconcile_without_deleting_task() {
    let (state, recorder) = setup(vec![event()]).await;
    capture(&state).await;
    let before = tasks(&state).await;
    let admissions = count(&state, "mutation_envelopes").await;
    recorder.delete_event(ORIGIN).await.unwrap();
    recorder.clear_recorded_calls();
    let result = reconcile(&state).await;
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(result["diagnostics"][0]["code"], "capture_source_removed");
    assert!(result["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains(before[0]["id"].as_str().unwrap()));
    assert_eq!(tasks(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    println!("EVIDENCE[test8_diagnostics]={}", result["diagnostics"]);
    // Absence outside the observation range is not evidence of removal. Repair preserves that ownership.
    let tomorrow = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-26T08:00:00Z").unwrap(),
    ));
    let outside = reconcile(&tomorrow).await;
    assert_eq!(outside["diagnostics"], json!([]));
    assert_eq!(outside["conflicts"], json!([]));
    ok(
        &tomorrow,
        "POST",
        &format!(
            "/projection/calendar/reconcile/{}/repair",
            outside["reconciliation_id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await;
    assert_eq!(
        last_applied_events(state.inner().store.pool())
            .await
            .unwrap()
            .len(),
        1
    );
}
