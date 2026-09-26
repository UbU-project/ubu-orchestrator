//! Synthetic HTTP and pure-decision evidence. No Calendar transport is constructed.
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_apply::last_applied_events,
        calendar_client::{CalendarApi, RecordedCalendarCall, RecordingCalendarApi},
        calendar_interaction::{detect, observed_owned},
        calendar_projection::DesiredEvent,
    },
    state::AppState,
};
use ubu_store::{models::calendar_record::NewCalendarRecord, queries};
const NOW: &str = "2026-09-25T08:00:00Z";
const END: &str = "2026-09-25T20:00:00Z";
const ORIGIN: &str = "5n0q8c9h7g4k2m1p3r6t8v0a2c";
async fn state(recorder: Arc<RecordingCalendarApi>) -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder)
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
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
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}
async fn add(state: &AppState, mut fields: Value) -> String {
    fields["schema_version"] = "ubu.orchestrator.task_capture.v1".into();
    ok(state, "POST", "/task", fields).await["task_id"]
        .as_str()
        .unwrap()
        .into()
}
fn dynamic() -> Value {
    json!({"title":"Synthetic Dynamic work","duration_estimate":{"type":"fixed","seconds":1800},"tags":["personal"],"category_tag":"personal"})
}
fn fixed() -> Value {
    json!({"title":"Synthetic Static work","static_window":{"start":"2026-09-25T12:00:00Z","end":"2026-09-25T12:30:00Z"},"tags":["work"],"category_tag":"work"})
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
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, p: &Value) {
    let result=ok(state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await;
    assert_eq!(result["status"], "applied");
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
async fn setup(fields: Value) -> (AppState, Arc<RecordingCalendarApi>, String) {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state(recorder.clone()).await;
    let id = add(&state, fields).await;
    generate(&state).await;
    approve(&state, &preview(&state).await).await;
    recorder.clear_recorded_calls();
    (state, recorder, id)
}
fn event(recorder: &RecordingCalendarApi, id: &str) -> DesiredEvent {
    recorder
        .events()
        .into_iter()
        .find(|e| e.task_id == id)
        .unwrap()
}
async fn phone(recorder: &RecordingCalendarApi, id: &str, color: Option<&str>, actual: bool) {
    let mut e = event(recorder, id);
    e.color_id = color.map(str::to_owned);
    if actual {
        e.start_at = "2026-09-25T09:05:00Z".into();
        e.end_at = "2026-09-25T09:50:00Z".into();
    }
    recorder.patch_event(&e).await.unwrap();
    recorder.clear_recorded_calls();
}
async fn task(state: &AppState, id: &str) -> Value {
    let row = queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    json!({"version":row.version,"status":row.status,"payload":serde_json::from_str::<Value>(&row.payload_json).unwrap()})
}
async fn actions(state: &AppState, id: &str) -> Vec<Value> {
    let rows:Vec<(String,String,String,String,String,String)>=sqlx::query_as("SELECT id,event_type,payload_json,object_refs_json,provenance_json,created_at FROM logs WHERE EXISTS (SELECT 1 FROM json_each(logs.object_refs_json) WHERE value=?) AND (event_type='task_done' OR json_extract(payload_json,'$.action') IN ('complete','reopen')) ORDER BY rowid").bind(id).fetch_all(state.inner().store.pool()).await.unwrap();
    rows.into_iter().map(|(id,event_type,payload,refs,provenance,created_at)|json!({"id":id,"event_type":event_type,"payload":serde_json::from_str::<Value>(&payload).unwrap(),"object_refs":serde_json::from_str::<Value>(&refs).unwrap(),"provenance":serde_json::from_str::<Value>(&provenance).unwrap(),"created_at":created_at})).collect()
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn universe(state: &AppState) -> Value {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='UniverseState'")
            .fetch_optional(state.inner().store.pool())
            .await
            .unwrap();
    raw.map(|raw| serde_json::from_str(&raw).unwrap())
        .unwrap_or(Value::Null)
}
fn counts(response: &Value) -> Value {
    json!({"captured":response["captured"],"updated":response["updated"],"unchanged":response["unchanged"],"skipped":response["skipped"],"diagnostics":response["diagnostics"]})
}

#[tokio::test]
async fn dynamic_and_static_preview_colours_follow_the_partition() {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state(recorder.clone()).await;
    let d = add(&state, dynamic()).await;
    let s = add(&state, fixed()).await;
    generate(&state).await;
    let p = preview(&state).await;
    let events = p["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().find(|e| e["task_id"] == d).unwrap()["color_id"].is_null());
    assert_eq!(
        events.iter().find(|e| e["task_id"] == s).unwrap()["color_id"],
        "9"
    );
    let calendar = ok(&state, "GET", "/calendar/current", Value::Null).await;
    assert_eq!(
        calendar["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["task_id"] == d)
            .unwrap()["category_tag"],
        "personal"
    );
    approve(&state, &p).await;
    let mut applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    let dynamic_event = applied.iter().find(|event| event.task_id == d).unwrap();
    use ubu_orchestrator::services::calendar_wire::{event_request, Operation, CALENDAR_API_BASE};
    let patch = event_request(
        Operation::Patch,
        CALENDAR_API_BASE,
        "synthetic-calendar",
        dynamic_event,
    )
    .body
    .unwrap();
    let insert = event_request(
        Operation::Insert,
        CALENDAR_API_BASE,
        "synthetic-calendar",
        dynamic_event,
    )
    .body
    .unwrap();
    assert_eq!(patch.get("colorId"), Some(&Value::Null));
    assert!(insert.get("colorId").is_none());
    let old = applied.iter_mut().find(|e| e.task_id == d).unwrap();
    old.color_id = Some("3".into());
    let owned = observed_owned(state.inner().store.pool(), &applied, &applied)
        .await
        .unwrap();
    let (complete, reopen, diagnostics) = detect(&owned, UbuTimestamp::parse(NOW).unwrap());
    assert!(complete.is_empty() && reopen.is_empty() && diagnostics.is_empty());
}

#[tokio::test]
async fn phone_colour_completes_with_observed_window_and_source_log() {
    let (state, recorder, id) = setup(dynamic()).await;
    add(&state, fixed()).await;
    generate(&state).await;
    approve(&state, &preview(&state).await).await;
    let planned = event(&recorder, &id);
    phone(&recorder, &id, Some("11"), true).await;
    let response = capture(&state).await;
    assert_eq!(response["updated"], 1);
    assert_eq!(response["skipped"], 0);
    let stored = task(&state, &id).await;
    assert_eq!(stored["status"], "completed");
    assert!(stored["payload"].get("static_window").is_none());
    assert_eq!(
        stored["payload"]["duration_estimate"],
        json!({"type":"fixed","seconds":2700})
    );
    let logs = actions(&state, &id).await;
    assert_eq!(logs.len(), 1);
    let payload = &logs[0]["payload"];
    assert_eq!(
        payload["source"],
        json!({"source_kind":"google_calendar","source_id":planned.external_id})
    );
    assert_eq!(
        payload["observed_window"],
        json!({"start":"2026-09-25T09:05:00Z","end":"2026-09-25T09:50:00Z"})
    );
    assert_ne!(payload["observed_window"]["start"], planned.start_at);
    assert_eq!(payload["decision"], "task_completed");
    assert_eq!(payload["transition_applied"], true);
    println!("EVIDENCE[P1B33_test2_log_payload]={payload}");
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    assert_eq!(preview(&state).await["operations"], json!([]));
    generate(&state).await;
    let after = preview(&state).await;
    assert_eq!(after["operations"], json!([]));
    approve(&state, &after).await;
    assert_eq!(event(&recorder, &id).color_id.as_deref(), Some("11"));
}

#[tokio::test]
async fn calendar_completion_applies_the_same_effects_as_app_completion() {
    let mut fields = dynamic();
    fields["effects"] = json!({"success_probability":0.01,"mutations":[{"operation":"set_fact","target":"facts.synthetic_done","payload":true},{"operation":"increment_numeric","target":"numeric_values.synthetic_completions","payload":1.0}]});
    let (state, recorder, id) = setup(fields.clone()).await;
    let before = universe(&state).await;
    assert!(before.is_null());
    phone(&recorder, &id, Some("2"), true).await;
    capture(&state).await;
    let after = universe(&state).await;
    assert_eq!(after["facts"]["synthetic_done"], true);
    assert_eq!(after["numeric_values"]["synthetic_completions"], 1.0);
    let (app, _, app_id) = setup(fields).await;
    ok(
        &app,
        "POST",
        &format!("/task/{app_id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    let expected = universe(&app).await;
    assert_eq!(after["facts"], expected["facts"]);
    assert_eq!(after["numeric_values"], expected["numeric_values"]);
    assert_eq!(task(&app, &app_id).await["status"], "completed");
    println!(
        "EVIDENCE[P1B33_test3_universe_change]={}",
        json!({"before":before,"after":after})
    );
}

#[tokio::test]
async fn removing_colour_reopens_recent_calendar_completion_with_linked_log() {
    let (state, recorder, id) = setup(dynamic()).await;
    phone(&recorder, &id, Some("8"), true).await;
    capture(&state).await;
    let completed = task(&state, &id).await;
    let completion = actions(&state, &id).await.remove(0);
    let later = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-25T10:00:00Z").unwrap(),
    ));
    phone(&recorder, &id, None, false).await;
    let response = capture(&later).await;
    assert_eq!(response["updated"], 1);
    assert_eq!(response["skipped"], 0);
    let reopened = task(&state, &id).await;
    assert_eq!(reopened["status"], "active");
    assert!(reopened["version"].as_i64() > completed["version"].as_i64());
    let logs = actions(&state, &id).await;
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[1]["payload"]["decision"], "task_reopened");
    assert_eq!(logs[1]["payload"]["completion_log_id"], completion["id"]);
    assert_eq!(logs[1]["object_refs"], json!([id, completion["id"]]));
    println!("EVIDENCE[P1B33_test4_logs]={}", json!(logs));
    let again = capture(&later).await;
    assert_eq!(again["updated"], 0);
    assert_eq!(again["unchanged"], 1);
    assert_eq!(actions(&state, &id).await, logs);
}

#[tokio::test]
async fn app_completion_is_never_reopened_including_a_newer_legacy_done_fact() {
    let (state, recorder, id) = setup(dynamic()).await;
    ok(
        &state,
        "POST",
        &format!("/task/{id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    let before = task(&state, &id).await;
    let logs = actions(&state, &id).await;
    let log_count = count(&state, "logs").await;
    phone(&recorder, &id, None, false).await;
    let response = capture(&state).await;
    assert_eq!(response["updated"], 0);
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(actions(&state, &id).await, logs);
    assert_eq!(count(&state, "logs").await, log_count);
    let (other, phone_api, other_id) = setup(dynamic()).await;
    phone(&phone_api, &other_id, Some("3"), true).await;
    capture(&other).await;
    ok(&other, "POST", &format!("/task/{other_id}/done"), json!({})).await;
    let before = task(&other, &other_id).await;
    let logs = actions(&other, &other_id).await;
    assert_eq!(logs.last().unwrap()["event_type"], "task_done");
    phone(&phone_api, &other_id, None, false).await;
    assert_eq!(capture(&other).await["updated"], 0);
    assert_eq!(task(&other, &other_id).await, before);
    assert_eq!(actions(&other, &other_id).await, logs);
    println!(
        "EVIDENCE[P1B33_test5_app_unchanged]={}",
        json!({"task_unchanged":true,"logs_unchanged":true,"newer_legacy_done_protected":true})
    );
}

#[tokio::test]
async fn twenty_five_hour_old_events_do_not_reopen_even_when_observed() {
    let (state, recorder, id) = setup(dynamic()).await;
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: "2026-09-23T00:00:00Z".into(),
            window_end: END.into(),
            payload: json!({"windows":[]}),
            created_at: NOW.into(),
        },
    )
    .await
    .unwrap();
    let mut old = event(&recorder, &id);
    old.start_at = "2026-09-24T06:30:00Z".into();
    old.end_at = "2026-09-24T07:00:00Z".into();
    old.color_id = Some("4".into());
    recorder.patch_event(&old).await.unwrap();
    assert_eq!(capture(&state).await["updated"], 1);
    old.color_id = None;
    recorder.patch_event(&old).await.unwrap();
    let before = task(&state, &id).await;
    let logs = actions(&state, &id).await;
    assert_eq!(capture(&state).await["updated"], 0);
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(actions(&state, &id).await, logs);
    let applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    let mut owned = observed_owned(state.inner().store.pool(), &[old], &applied)
        .await
        .unwrap();
    assert_eq!(owned.len(), 1);
    owned[0].event.start_at = "2026-09-24T07:30:00Z".into();
    owned[0].event.end_at = "2026-09-24T08:00:00Z".into();
    assert!(detect(&owned, UbuTimestamp::parse(NOW).unwrap())
        .1
        .is_empty());
    owned[0].event.end_at = "2026-09-24T08:00:01Z".into();
    assert_eq!(detect(&owned, UbuTimestamp::parse(NOW).unwrap()).1.len(), 1);
}

#[tokio::test]
async fn coloured_static_events_do_not_complete_tasks() {
    let (state, recorder, id) = setup(fixed()).await;
    phone(&recorder, &id, Some("7"), false).await;
    let before = task(&state, &id).await;
    let response = capture(&state).await;
    assert_eq!(response["updated"], 0);
    assert_eq!(task(&state, &id).await, before);
    assert!(actions(&state, &id).await.is_empty());
    let applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    let mut owned = observed_owned(state.inner().store.pool(), &recorder.events(), &applied)
        .await
        .unwrap();
    owned[0].exported_uncoloured = true;
    let (complete, reopen, diagnostics) = detect(&owned, UbuTimestamp::parse(NOW).unwrap());
    assert!(complete.is_empty() && reopen.is_empty() && diagnostics.is_empty());
}

#[tokio::test]
async fn coloured_captured_events_do_not_complete_tasks() {
    let source = DesiredEvent {
        external_id: ORIGIN.into(),
        task_id: format!("task_{ORIGIN}"),
        summary: "Synthetic captured appointment".into(),
        start_at: "2026-09-25T09:00:00Z".into(),
        end_at: "2026-09-25T09:30:00Z".into(),
        color_id: Some("3".into()),
        transparent: false,
        reminders_minutes: vec![],
    };
    let recorder = Arc::new(RecordingCalendarApi::with_events([source]));
    let state = state(recorder.clone()).await;
    assert_eq!(capture(&state).await["captured"], 1);
    let applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    let id = applied[0].task_id.clone();
    let mut observed = recorder.events();
    observed[0].color_id = Some("11".into());
    recorder.patch_event(&observed[0]).await.unwrap();
    let before = task(&state, &id).await;
    assert_eq!(capture(&state).await["updated"], 0);
    assert_eq!(task(&state, &id).await, before);
    assert!(actions(&state, &id).await.is_empty());
    let mut owned = observed_owned(state.inner().store.pool(), &observed, &applied)
        .await
        .unwrap();
    owned[0].is_static = false;
    owned[0].exported_uncoloured = true;
    let (complete, reopen, diagnostics) = detect(&owned, UbuTimestamp::parse(NOW).unwrap());
    assert!(complete.is_empty() && reopen.is_empty() && diagnostics.is_empty());
}

#[tokio::test]
async fn capture_distinguishes_unchanged_updated_and_named_owned_drift() {
    let (state, recorder, id) = setup(dynamic()).await;
    let unchanged = capture(&state).await;
    assert_eq!(
        counts(&unchanged),
        json!({"captured":0,"updated":0,"unchanged":1,"skipped":0,"diagnostics":[]})
    );
    let mut changed = event(&recorder, &id);
    changed.summary = "Synthetic phone title edit".into();
    recorder.patch_event(&changed).await.unwrap();
    let drift = capture(&state).await;
    assert_eq!(drift["updated"], 0);
    assert_eq!(drift["unchanged"], 0);
    assert_eq!(drift["skipped"], 0);
    assert_eq!(drift["diagnostics"][0]["code"], "capture_owned_drift");
    let message = drift["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains(&id) && message.contains(&changed.external_id));
    assert_eq!(
        task(&state, &id).await["payload"]["title"],
        dynamic()["title"]
    );
    phone(&recorder, &id, Some("6"), false).await;
    let updated = capture(&state).await;
    assert_eq!(
        counts(&updated),
        json!({"captured":0,"updated":1,"unchanged":0,"skipped":0,"diagnostics":[]})
    );
    println!(
        "EVIDENCE[P1B33_test9_counts]={}",
        json!({"unchanged":counts(&unchanged),"updated":counts(&updated),"drift":counts(&drift)})
    );
}

#[tokio::test]
async fn an_already_completed_coloured_event_is_a_noop_without_another_log_or_effect() {
    let mut fields = dynamic();
    fields["effects"] = json!({"mutations":[{"operation":"increment_numeric","target":"numeric_values.synthetic_completions","payload":1.0}]});
    let (state, recorder, id) = setup(fields).await;
    phone(&recorder, &id, Some("10"), true).await;
    assert_eq!(capture(&state).await["updated"], 1);
    let before = task(&state, &id).await;
    let logs = count(&state, "logs").await;
    let mutations = count(&state, "mutation_envelopes").await;
    let results = count(&state, "projection_results").await;
    let effects = universe(&state).await;
    let response = capture(&state).await;
    assert_eq!(response["updated"], 0);
    assert_eq!(response["unchanged"], 1);
    assert_eq!(response["diagnostics"], json!([]));
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(count(&state, "logs").await, logs);
    assert_eq!(count(&state, "mutation_envelopes").await, mutations);
    assert_eq!(count(&state, "projection_results").await, results);
    assert_eq!(actions(&state, &id).await.len(), 1);
    assert_eq!(universe(&state).await, effects);
}
