//! Synthetic HTTP and pure-decision evidence. No Calendar transport is constructed.
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_apply::last_applied_events,
        calendar_client::{CalendarApi, RecordedCalendarCall, RecordingCalendarApi},
        calendar_interaction::{detect_window_gestures, observed_owned},
        calendar_projection::DesiredEvent,
    },
    state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};
const NOW: &str = "2026-09-26T08:00:00Z";
const END: &str = "2026-09-26T20:00:00Z";
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
    json!({"title":"Synthetic Static work","static_window":{"start":"2026-09-26T12:00:00Z","end":"2026-09-26T12:30:00Z"},"tags":["work"],"category_tag":"work"})
}
async fn generate(state: &AppState) -> Value {
    let result = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":END}}),
    )
    .await;
    assert!(result["plan"].is_object(), "{result}");
    result
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
async fn task(state: &AppState, id: &str) -> Value {
    let row = queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    json!({"version":row.version,"status":row.status,"payload":serde_json::from_str::<Value>(&row.payload_json).unwrap()})
}

async fn drag(
    recorder: &RecordingCalendarApi,
    id: &str,
    start: &str,
    end: &str,
    color: Option<&str>,
) {
    let mut e = event(recorder, id);
    e.start_at = start.into();
    e.end_at = end.into();
    if let Some(color) = color {
        e.color_id = Some(color.into());
    }
    recorder.patch_event(&e).await.unwrap();
    recorder.clear_recorded_calls();
}
async fn reconcile(state: &AppState) -> Value {
    ok(state,"POST","/projection/calendar/reconcile",json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"})).await
}
async fn repair(state: &AppState, r: &Value) {
    ok(
        state,
        "POST",
        &format!(
            "/projection/calendar/reconcile/{}/repair",
            r["reconciliation_id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await;
}
async fn signals(state: &AppState, recorder: &RecordingCalendarApi) -> Value {
    let applied = last_applied_events(state.inner().store.pool())
        .await
        .unwrap();
    let owned = observed_owned(state.inner().store.pool(), &recorder.events(), &applied)
        .await
        .unwrap();
    let (moves, resizes, diagnostics) = detect_window_gestures(&owned);
    json!({"moves":moves,"resizes":resizes,"diagnostics":diagnostics})
}
async fn logs(state: &AppState, id: &str) -> Vec<Value> {
    let rows: Vec<(String,String,String,String,String,String)> = sqlx::query_as("SELECT id,event_type,payload_json,object_refs_json,provenance_json,created_at FROM logs WHERE EXISTS (SELECT 1 FROM json_each(logs.object_refs_json) WHERE value=?) ORDER BY rowid").bind(id).fetch_all(state.inner().store.pool()).await.unwrap();
    rows.into_iter().map(|(id,event_type,payload,refs,provenance,created_at)|json!({"id":id,"event_type":event_type,"payload":serde_json::from_str::<Value>(&payload).unwrap(),"object_refs":serde_json::from_str::<Value>(&refs).unwrap(),"provenance":serde_json::from_str::<Value>(&provenance).unwrap(),"created_at":created_at})).collect()
}
fn has_code(response: &Value, code: &str) -> bool {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}
async fn meeting() -> (AppState, Arc<RecordingCalendarApi>, String) {
    let original = DesiredEvent {
        external_id: ORIGIN.into(),
        task_id: String::new(),
        summary: "Dentist".into(),
        start_at: "2026-09-26T14:00:00Z".into(),
        end_at: "2026-09-26T14:30:00Z".into(),
        color_id: None,
        transparent: false,
        reminders_minutes: vec![],
    };
    let recorder = Arc::new(RecordingCalendarApi::with_events(vec![original]));
    let state = state(recorder.clone()).await;
    assert_eq!(capture(&state).await["captured"], 1);
    let id: String = sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task'")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    generate(&state).await;
    approve(&state, &preview(&state).await).await;
    // The recording client retains the original wire identity when apply is a no-op.
    let mut owned = recorder.events().remove(0);
    owned.task_id = id.clone();
    recorder.patch_event(&owned).await.unwrap();
    recorder.clear_recorded_calls();
    (state, recorder, id)
}
const ROUTINE: &str = "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01";
async fn admit_routine(state: &AppState) {
    let now = state.planning_now();
    let id = UbuId::parse(ROUTINE).unwrap();
    let payload = json!({"id":id,"status":"active","title":"Synthetic daily routine","mode":"evergreen","recurrence":{"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1},"routine_instance_template":{"title":"Synthetic routine occurrence","nominal_start":"09:00:00","placement":"planned","allowed_local_range":{"earliest":"09:00:00","latest":"12:00:00"},"duration_estimate":{"type":"fixed","seconds":600},"template_version":1},"provenance":{"created_at":now,"authority_source":"user"}});
    let envelope = state
        .envelope_for(
            [(id, VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: ROUTINE.into(),
            object_type: ObjectType::Objective.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "synthetic".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
async fn active_occurrence(state: &AppState) -> String {
    sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=?").bind(ROUTINE).fetch_one(state.inner().store.pool()).await.unwrap()
}
async fn action(state: &AppState, id: &str, action: &str) {
    ok(
        state,
        "POST",
        &format!("/task/{id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":action}),
    )
    .await;
}

#[tokio::test]
async fn captured_meeting_follows_drag_and_next_preview_has_no_operation() {
    let (state, recorder, id) = meeting().await;
    drag(
        &recorder,
        &id,
        "2026-09-26T16:00:00Z",
        "2026-09-26T16:30:00Z",
        None,
    )
    .await;
    let r = reconcile(&state).await;
    assert_eq!(r["conflicts"][0]["conflict_type"], "drifted");
    repair(&state, &r).await;
    let before_task = task(&state, &id).await;
    let before_preview = preview(&state).await;
    assert_eq!(
        before_preview["operations"][0]["event"]["start_at"],
        "2026-09-26T14:00:00Z"
    );
    println!(
        "EVIDENCE[P1B34_test1_before]={}",
        json!({"static_window":before_task["payload"]["static_window"],"operations":before_preview["operations"]})
    );
    let response = capture(&state).await;
    assert_eq!(response["moved"], 1);
    assert_eq!(response["resized"], 0);
    let after_task = task(&state, &id).await;
    assert_eq!(
        after_task["payload"]["static_window"],
        json!({"start":"2026-09-26T16:00:00Z","end":"2026-09-26T16:30:00Z"})
    );
    assert_eq!(
        after_task["version"],
        before_task["version"].as_i64().unwrap() + 1
    );
    let after_preview = preview(&state).await;
    assert_eq!(after_preview["operations"], json!([]));
    println!(
        "EVIDENCE[P1B34_test1_after]={}",
        json!({"static_window":after_task["payload"]["static_window"],"operations":after_preview["operations"]})
    );
    generate(&state).await;
    assert_eq!(preview(&state).await["operations"], json!([]));
    let repeat = capture(&state).await;
    assert_eq!(repeat["moved"], 0);
    assert_eq!(repeat["unchanged"], 1);
    // App-authored offset timestamps name the same instants, not another move.
    ok(&state, "PATCH", &format!("/task/{id}"), json!({
        "schema_version":"ubu.orchestrator.task_capture.v1", "expected_version":after_task["version"],
        "static_window":{"start":"2026-09-26T18:00:00+02:00","end":"2026-09-26T18:30:00+02:00"}
    })).await;
    let offset_task = task(&state, &id).await;
    assert_eq!(signals(&state, &recorder).await["moves"], json!([]));
    assert_eq!(capture(&state).await["moved"], 0);
    assert_eq!(task(&state, &id).await, offset_task);
}

#[tokio::test]
async fn reconcile_still_classifies_unimported_drag_as_drifted() {
    let (state, recorder, id) = meeting().await;
    let before = task(&state, &id).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T16:00:00Z",
        "2026-09-26T16:30:00Z",
        None,
    )
    .await;
    let r = reconcile(&state).await;
    assert_eq!(r["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(r["conflicts"][0]["conflict_type"], "drifted");
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(capture(&state).await["moved"], 1);
    assert_eq!(reconcile(&state).await["conflicts"], json!([]));
    assert_eq!(preview(&state).await["operations"], json!([]));
}

#[tokio::test]
async fn dynamic_resize_changes_declaration_without_changing_planned_position() {
    let (state, recorder, id) = setup(dynamic()).await;
    let planned = event(&recorder, &id);
    drag(
        &recorder,
        &id,
        "2026-09-26T13:00:00Z",
        "2026-09-26T13:45:00Z",
        None,
    )
    .await;
    let response = capture(&state).await;
    assert_eq!(response["resized"], 1);
    assert_eq!(response["moved"], 0);
    let t = task(&state, &id).await;
    assert_eq!(
        t["payload"]["duration_estimate"],
        json!({"type":"fixed","seconds":2700})
    );
    assert!(t["payload"].get("static_window").is_none());
    let current = ok(&state, "GET", "/calendar/current", Value::Null).await;
    let step = current["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap();
    assert_eq!(step["start_at"], planned.start_at);
    assert_eq!(step["end_at"], planned.end_at);
    generate(&state).await;
    let p = preview(&state).await;
    let e = p["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["task_id"] == id)
        .unwrap();
    assert_eq!(e["start_at"], planned.start_at);
    assert_eq!(
        UbuTimestamp::parse(e["end_at"].as_str().unwrap())
            .unwrap()
            .inner()
            .unix_timestamp()
            - UbuTimestamp::parse(e["start_at"].as_str().unwrap())
                .unwrap()
                .inner()
                .unix_timestamp(),
        2700
    );
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
}

#[tokio::test]
async fn dynamic_position_alone_is_not_a_signal_and_repair_projects_planned_slot() {
    let (state, recorder, id) = setup(dynamic()).await;
    let planned = event(&recorder, &id);
    let before = task(&state, &id).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T13:00:00Z",
        "2026-09-26T13:30:00Z",
        None,
    )
    .await;
    let detected = signals(&state, &recorder).await;
    assert_eq!(detected, json!({"moves":[],"resizes":[],"diagnostics":[]}));
    let response = capture(&state).await;
    assert_eq!(response["moved"], 0);
    assert_eq!(response["resized"], 0);
    assert_eq!(task(&state, &id).await, before);
    let r = reconcile(&state).await;
    repair(&state, &r).await;
    let p = preview(&state).await;
    assert_eq!(p["operations"].as_array().unwrap().len(), 1);
    assert_eq!(p["operations"][0]["kind"], "update");
    assert_eq!(p["operations"][0]["event"]["start_at"], planned.start_at);
    println!(
        "EVIDENCE[P1B34_test4]={}",
        json!({"signals":detected,"diagnostics":response["diagnostics"],"operations":p["operations"]})
    );
}

#[tokio::test]
async fn occurrence_drag_requires_override_and_does_not_edit_task() {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state(recorder.clone()).await;
    admit_routine(&state).await;
    generate(&state).await;
    let id = active_occurrence(&state).await;
    approve(&state, &preview(&state).await).await;
    let before = task(&state, &id).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T10:00:00Z",
        "2026-09-26T10:10:00Z",
        None,
    )
    .await;
    let detected = signals(&state, &recorder).await;
    assert_eq!(detected["moves"], json!([]));
    assert_eq!(detected["resizes"], json!([]));
    let response = capture(&state).await;
    assert!(has_code(
        &response,
        "calendar_move_needs_occurrence_override"
    ));
    let message = response["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains(&id) && message.contains(ROUTINE));
    assert_eq!(task(&state, &id).await, before);
    println!(
        "EVIDENCE[P1B34_test5]={}",
        json!({"signals":detected,"diagnostics":response["diagnostics"]})
    );
}

#[tokio::test]
async fn observed_routine_resize_reports_model_priority_without_bypassing_override() {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state(recorder.clone()).await;
    let first = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-21T08:00:00Z").unwrap(),
    ));
    admit_routine(&first).await;
    for day in 21..=25 {
        let start = format!("2026-09-{day}T09:00:00Z");
        let end = format!("2026-09-{day}T09:20:00Z");
        let earlier = state
            .clone()
            .with_clock(FixedClock(UbuTimestamp::parse(&start).unwrap()));
        ok(
            &earlier,
            "POST",
            "/planning/generate",
            json!({"horizon":{"start":start,"end":format!("2026-09-{day}T12:00:00Z")}}),
        )
        .await;
        let id = active_occurrence(&earlier).await;
        action(&earlier, &id, "start").await;
        let completed = earlier.with_clock(FixedClock(UbuTimestamp::parse(&end).unwrap()));
        action(&completed, &id, "complete").await;
    }
    let plan = generate(&state).await;
    assert!(has_code(&plan, "duration_model_observed"));
    let id = active_occurrence(&state).await;
    approve(&state, &preview(&state).await).await;
    let initial = event(&recorder, &id);
    let before = task(&state, &id).await;
    assert_eq!(
        UbuTimestamp::parse(&initial.end_at)
            .unwrap()
            .inner()
            .unix_timestamp()
            - UbuTimestamp::parse(&initial.start_at)
                .unwrap()
                .inner()
                .unix_timestamp(),
        1200
    );
    // A model-sized event that has not changed is not a rejected phone gesture.
    assert_eq!(capture(&state).await["diagnostics"], json!([]));
    drag(
        &recorder,
        &id,
        "2026-09-26T09:00:00Z",
        "2026-09-26T09:25:00Z",
        None,
    )
    .await;
    let detected = signals(&state, &recorder).await;
    let response = capture(&state).await;
    assert!(has_code(
        &response,
        "calendar_move_needs_occurrence_override"
    ));
    assert!(has_code(
        &response,
        "calendar_resize_overridden_by_observations"
    ));
    let diagnostic = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "calendar_resize_overridden_by_observations")
        .unwrap();
    assert!(diagnostic["message"].as_str().unwrap().contains("5"));
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(response["resized"], 0);
    generate(&state).await;
    let p = preview(&state).await;
    let projected = p["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["task_id"] == id)
        .unwrap();
    assert_eq!(projected["end_at"], initial.end_at);
    println!(
        "EVIDENCE[P1B34_test6]={}",
        json!({"signals":detected,"diagnostics":response["diagnostics"],"declared_duration":before["payload"]["duration_estimate"],"planned_start":projected["start_at"],"planned_end":projected["end_at"]})
    );
}

#[tokio::test]
async fn all_three_move_sanity_guards_reject_without_writes() {
    let (state, recorder, id) = setup(fixed()).await;
    for (start, end, code) in [
        (
            "2026-09-26T16:00:00Z",
            "2026-09-26T16:00:00Z",
            "calendar_move_invalid_window",
        ),
        (
            "2026-09-26T07:00:00Z",
            "2026-09-26T08:30:00Z",
            "calendar_move_before_creation",
        ),
        (
            "2026-09-26T16:00:00Z",
            "2026-10-26T17:00:00Z",
            "calendar_move_beyond_horizon",
        ),
    ] {
        let before = task(&state, &id).await;
        let before_logs = logs(&state, &id).await;
        let before_admissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
        drag(&recorder, &id, start, end, None).await;
        let response = capture(&state).await;
        assert!(has_code(&response, code), "{response}");
        assert_eq!(response["updated"], 0);
        assert_eq!(response["moved"], 0);
        assert_eq!(task(&state, &id).await, before);
        assert_eq!(logs(&state, &id).await, before_logs);
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
        assert_eq!(after, before_admissions);
        println!("EVIDENCE[P1B34_test7_{code}]={}", response["diagnostics"]);
    }
}

#[tokio::test]
async fn completed_task_gesture_is_ignored_visibly() {
    let (state, recorder, id) = setup(dynamic()).await;
    action(&state, &id, "complete").await;
    let before = task(&state, &id).await;
    let before_logs = logs(&state, &id).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T13:00:00Z",
        "2026-09-26T13:45:00Z",
        None,
    )
    .await;
    let response = capture(&state).await;
    assert!(has_code(&response, "calendar_gesture_on_inactive_task"));
    assert_eq!(response["updated"], 0);
    assert_eq!(response["moved"], 0);
    assert_eq!(response["resized"], 0);
    assert_eq!(task(&state, &id).await, before);
    assert_eq!(logs(&state, &id).await, before_logs);
}

#[tokio::test]
async fn dynamic_drag_resize_and_colour_apply_before_completion_with_observed_window() {
    let (state, recorder, id) = setup(dynamic()).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T13:00:00Z",
        "2026-09-26T13:45:00Z",
        Some("11"),
    )
    .await;
    let response = capture(&state).await;
    assert_eq!(response["updated"], 1);
    assert_eq!(response["resized"], 1);
    assert_eq!(response["moved"], 0);
    let t = task(&state, &id).await;
    assert_eq!(t["status"], "completed");
    assert_eq!(t["version"], 3);
    assert_eq!(
        t["payload"]["duration_estimate"],
        json!({"type":"fixed","seconds":2700})
    );
    assert!(t["payload"].get("static_window").is_none());
    let entries = logs(&state, &id).await;
    let completion = entries
        .iter()
        .find(|l| l["payload"]["decision"] == "task_completed")
        .unwrap();
    assert_eq!(
        completion["payload"]["observed_window"],
        json!({"start":"2026-09-26T13:00:00Z","end":"2026-09-26T13:45:00Z"})
    );
    assert_eq!(preview(&state).await["operations"], json!([]));
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    // Static colour remains its category: a moved coloured meeting stays active.
    let (s, r, id) = setup(fixed()).await;
    drag(
        &r,
        &id,
        "2026-09-26T16:00:00Z",
        "2026-09-26T16:30:00Z",
        Some("11"),
    )
    .await;
    assert_eq!(capture(&s).await["moved"], 1);
    assert_eq!(task(&s, &id).await["status"], "active");
}

#[tokio::test]
async fn calendar_move_log_has_source_marker_but_app_edit_does_not() {
    let (state, recorder, id) = setup(fixed()).await;
    drag(
        &recorder,
        &id,
        "2026-09-26T16:00:00Z",
        "2026-09-26T16:30:00Z",
        None,
    )
    .await;
    capture(&state).await;
    let entries = logs(&state, &id).await;
    let moved = entries
        .iter()
        .find(|l| l["payload"]["action"] == "move")
        .unwrap();
    assert_eq!(
        moved["payload"]["source"],
        json!({"source_kind":"google_calendar","source_id":event(&recorder,&id).external_id})
    );
    assert_eq!(moved["object_refs"], json!([id]));
    println!("EVIDENCE[P1B34_test10_log]={moved}");
    let current = task(&state, &id).await;
    ok(&state,"PATCH",&format!("/task/{id}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":current["version"],"static_window":{"start":"2026-09-26T17:00:00Z","end":"2026-09-26T17:30:00Z"}})).await;
    assert_eq!(
        task(&state, &id).await["payload"]["static_window"]["start"],
        "2026-09-26T17:00:00Z"
    );
    assert_eq!(logs(&state, &id).await, entries);
}

#[tokio::test]
async fn mixed_capture_reports_moves_resizes_and_unique_updated_tasks() {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state(recorder.clone()).await;
    let s = add(&state, fixed()).await;
    let d = add(&state, dynamic()).await;
    let untouched = add(&state, dynamic()).await;
    generate(&state).await;
    approve(&state, &preview(&state).await).await;
    let original = task(&state, &untouched).await;
    drag(
        &recorder,
        &s,
        "2026-09-26T16:00:00Z",
        "2026-09-26T16:30:00Z",
        None,
    )
    .await;
    drag(
        &recorder,
        &d,
        "2026-09-26T13:00:00Z",
        "2026-09-26T13:45:00Z",
        None,
    )
    .await;
    let response = capture(&state).await;
    assert_eq!(response["moved"], 1);
    assert_eq!(response["resized"], 1);
    assert_eq!(response["updated"], 2);
    assert_eq!(response["unchanged"], 1);
    assert_eq!(response["captured"], 0);
    assert_eq!(response["skipped"], 0);
    assert_eq!(response["diagnostics"], json!([]));
    assert_eq!(task(&state, &untouched).await, original);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
}
