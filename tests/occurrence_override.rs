//! Synthetic, in-process triage and RecordingCalendarApi evidence for P1B-35.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use ubu_core::{AuthoritySource, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_client::{CalendarApi, RecordedCalendarCall, RecordingCalendarApi},
        calendar_projection::DesiredEvent,
        routine_instantiation::{instantiate, RoutineDefinition},
    },
    state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};
const NOW: &str = "2026-09-27T06:00:00Z";
const END: &str = "2026-09-27T23:59:00Z";
const DAY: &str = "2026-09-27";
const A: &str = "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01";
const B: &str = "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e02";
const VERSION: &str = "ubu.orchestrator.routine_override.v1";
fn sec(time: &str) -> u64 {
    UbuTimestamp::parse(time).unwrap().inner().unix_timestamp() as u64
}
async fn state() -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder.clone());
    (state, recorder)
}
fn fields() -> Value {
    json!({"title":"Shower and dress","mode":"evergreen","recurrence":{"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1},"routine_instance_template":{"title":"Shower and dress","nominal_start":"07:00:00","placement":"planned","allowed_local_range":{"earliest":"07:00:00","latest":"12:00:00"},"duration_estimate":{"type":"fixed","seconds":1800},"template_version":1}})
}
async fn admit(state: &AppState, id: &str, fields: Value) {
    let now = state.planning_now();
    let id = UbuId::parse(id).unwrap();
    let mut payload = json!({"id":id,"status":"active","provenance":{"created_at":now,"authority_source":"user"}});
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: "Objective".into(),
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
async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = build_router(state.clone())
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
    let status = r.status();
    let body = serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}
fn path(id: &str, day: &str) -> String {
    format!("/routine/{id}/override/{day}")
}
async fn put(state: &AppState, id: &str, day: &str, start: &str, end: &str) -> Value {
    ok(
        state,
        "PUT",
        &path(id, day),
        json!({"schema_version":VERSION,"start":start,"end":end}),
    )
    .await
}
async fn pin(state: &AppState) -> Value {
    put(
        state,
        A,
        DAY,
        "2026-09-27T15:00:00Z",
        "2026-09-27T15:30:00Z",
    )
    .await
}
async fn generate_to(state: &AppState, end: &str) -> Value {
    let p = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":end}}),
    )
    .await;
    assert!(p["plan"].is_object(), "{p}");
    p
}
async fn generate(state: &AppState) -> Value {
    generate_to(state, END).await
}
async fn setup() -> (AppState, Arc<RecordingCalendarApi>) {
    let (s, r) = state().await;
    admit(&s, A, fields()).await;
    generate(&s).await;
    (s, r)
}
async fn object(state: &AppState, id: &str) -> Value {
    let r = queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    json!({"id":r.id,"version":r.version,"status":r.status,"payload":serde_json::from_str::<Value>(&r.payload_json).unwrap()})
}
async fn occurrence(state: &AppState, id: &str, day: &str) -> Value {
    let id:String=sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=? AND json_extract(payload_json,'$.occurrence.local_date')=? AND status!='moot'").bind(id).bind(day).fetch_one(state.inner().store.pool()).await.unwrap();
    object(state, &id).await
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, p: &Value) {
    let r=ok(state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await;
    assert_eq!(r["status"], "applied");
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
fn event(recorder: &RecordingCalendarApi, id: &str) -> DesiredEvent {
    recorder
        .events()
        .into_iter()
        .find(|e| e.task_id == id)
        .unwrap()
}
async fn drag(recorder: &RecordingCalendarApi, id: &str, color: Option<&str>) {
    let mut e = event(recorder, id);
    e.start_at = "2026-09-27T15:00:00Z".into();
    e.end_at = "2026-09-27T15:30:00Z".into();
    e.color_id = color.map(str::to_owned);
    recorder.patch_event(&e).await.unwrap();
    recorder.clear_recorded_calls();
}
fn code(body: &Value, name: &str) -> bool {
    body["diagnostics"]
        .as_array()
        .is_some_and(|items| items.iter().any(|d| d["code"] == name))
}
async fn counts(state: &AppState) -> (i64, i64, i64) {
    let p = state.inner().store.pool();
    (
        sqlx::query_scalar("SELECT COUNT(*) FROM objects")
            .fetch_one(p)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
            .fetch_one(p)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM logs")
            .fetch_one(p)
            .await
            .unwrap(),
    )
}
async fn definition(state: &AppState, id: &str) -> RoutineDefinition {
    let o = object(state, id).await;
    RoutineDefinition {
        objective_id: UbuId::parse(id).unwrap(),
        schedule: serde_json::from_value(o["payload"]["recurrence"].clone()).unwrap(),
        template: serde_json::from_value(o["payload"]["routine_instance_template"].clone())
            .unwrap(),
    }
}

#[tokio::test]
async fn one_date_override_moves_window_without_changing_occurrence_key() {
    let (s, _) = setup().await;
    let before = occurrence(&s, A, DAY).await;
    let response = pin(&s).await;
    assert_eq!(response["overridden"], true);
    let after = occurrence(&s, A, DAY).await;
    assert_eq!(before["id"], after["id"]);
    assert_eq!(
        before["payload"]["occurrence"]["key"],
        after["payload"]["occurrence"]["key"]
    );
    assert_eq!(
        object(&s, A).await["payload"]["recurrence"]["schedule_version"],
        1
    );
    assert_eq!(
        after["payload"]["static_window"],
        json!({"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"})
    );
    println!(
        "EVIDENCE[P1B35_test1]={}",
        json!({"key_before":before["payload"]["occurrence"]["key"],"key_after":after["payload"]["occurrence"]["key"],"window_before":before["payload"]["allowed_time_range"],"window_after":after["payload"]["static_window"]})
    );
}
#[tokio::test]
async fn override_survives_second_and_third_generate_without_churn() {
    let (s, _) = setup().await;
    pin(&s).await;
    let expected = occurrence(&s, A, DAY).await;
    for _ in 0..2 {
        generate(&s).await;
        assert_eq!(occurrence(&s, A, DAY).await, expected);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Task'")
        .fetch_one(s.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    // An unrelated template edit changes template identity, but not the dated decision.
    let d = definition(&s, A).await;
    let mut edited = d.clone();
    edited.template.title = "Renamed synthetic routine".into();
    edited.template.template_version += 1;
    edited.template.nominal_start = "08:00:00".into();
    let out = instantiate(&[edited], sec(NOW), sec(END));
    let o = out
        .occurrences
        .iter()
        .find(|o| o.local_date == DAY)
        .unwrap();
    assert_eq!(
        (o.start, o.end),
        (sec("2026-09-27T15:00:00Z"), sec("2026-09-27T15:30:00Z"))
    );
    assert!(o.overridden);
}
#[tokio::test]
async fn planned_override_emits_static_window_without_allowed_range() {
    let (s, _) = setup().await;
    put(&s, A, DAY, "2026-09-27T15:00:00Z", "2026-09-27T15:45:00Z").await;
    let p = generate(&s).await;
    let task = occurrence(&s, A, DAY).await;
    let payload = &task["payload"];
    assert!(payload.get("allowed_time_range").is_none());
    assert!(payload["occurrence"]["key"]
        .as_str()
        .unwrap()
        .contains("/planned/"));
    assert_eq!(
        payload["static_window"],
        json!({"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:45:00Z"})
    );
    assert_eq!(payload["duration_estimate"]["seconds"], 1800);
    let step = p["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["task_id"] == task["id"])
        .unwrap();
    assert_eq!(
        step["end"].as_u64().unwrap() - step["start"].as_u64().unwrap(),
        2700
    );
    println!("EVIDENCE[P1B35_test3]={payload}");
}
#[tokio::test]
async fn other_dates_of_the_same_routine_remain_unchanged() {
    let (s, _) = state().await;
    admit(&s, A, fields()).await;
    generate_to(&s, "2026-09-30T00:00:00Z").await;
    let first = occurrence(&s, A, DAY).await;
    let third = occurrence(&s, A, "2026-09-29").await;
    put(
        &s,
        A,
        "2026-09-28",
        "2026-09-28T15:00:00Z",
        "2026-09-28T15:30:00Z",
    )
    .await;
    generate_to(&s, "2026-09-30T00:00:00Z").await;
    assert_eq!(occurrence(&s, A, DAY).await, first);
    assert_eq!(occurrence(&s, A, "2026-09-29").await, third);
}
#[tokio::test]
async fn calendar_drag_saves_override_and_immediate_preview_has_no_operations() {
    let (s, r) = setup().await;
    let before = occurrence(&s, A, DAY).await;
    let id = before["id"].as_str().unwrap();
    approve(&s, &preview(&s).await).await;
    // A start fact must not prevent this explicit placement-only decision.
    ok(
        &s,
        "POST",
        &format!("/task/{id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"start"}),
    )
    .await;
    drag(&r, id, None).await;
    let response = capture(&s).await;
    assert_eq!(response["moved"], 1);
    assert_eq!(response["resized"], 0);
    let objective = object(&s, A).await;
    assert_eq!(
        objective["payload"]["recurrence"]["overrides"],
        json!([{"local_date":DAY,"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"}])
    );
    let p = preview(&s).await;
    assert_eq!(p["operations"], json!([]));
    assert_eq!(r.recorded_calls(), vec![RecordedCalendarCall::ListEvents]);
    generate(&s).await;
    assert_eq!(preview(&s).await["operations"], json!([]));
    let raw:String=sqlx::query_scalar("SELECT payload_json FROM logs WHERE json_extract(payload_json,'$.action')='set_occurrence_override'").fetch_one(s.inner().store.pool()).await.unwrap();
    let log: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        log["source"],
        json!({"source_kind":"google_calendar","source_id":event(&r,id).external_id})
    );
    println!(
        "EVIDENCE[P1B35_test5]={}",
        json!({"operations":p["operations"],"log":log})
    );
    // One observation retains the initial Dynamic colour meaning after pinning.
    let (s, r) = setup().await;
    let task = occurrence(&s, A, DAY).await;
    let id = task["id"].as_str().unwrap();
    approve(&s, &preview(&s).await).await;
    drag(&r, id, Some("11")).await;
    assert_eq!(capture(&s).await["moved"], 1);
    assert_eq!(object(&s, id).await["status"], "completed");
    let raw:String=sqlx::query_scalar("SELECT payload_json FROM logs WHERE json_extract(payload_json,'$.decision')='task_completed'").fetch_one(s.inner().store.pool()).await.unwrap();
    let log: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        log["observed_window"],
        json!({"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"})
    );
    assert_eq!(preview(&s).await["operations"], json!([]));
}
#[tokio::test]
async fn outside_declared_range_is_honoured_and_reported() {
    let (s, _) = setup().await;
    let result = put(&s, A, DAY, "2026-09-27T22:00:00Z", "2026-09-27T22:30:00Z").await;
    assert!(code(&result, "routine_override_outside_allowed_range"));
    let p = generate(&s).await;
    assert!(code(&p, "routine_override_outside_allowed_range"));
    assert_eq!(
        occurrence(&s, A, DAY).await["payload"]["static_window"]["start"],
        "2026-09-27T22:00:00Z"
    );
    println!("EVIDENCE[P1B35_test6]={}", result["diagnostics"]);
}
#[tokio::test]
async fn after_maximum_and_minimum_overrides_are_honoured_and_reported() {
    let (s, _) = state().await;
    let mut parent = fields();
    parent["routine_instance_template"]["placement"] = "static".into();
    parent["routine_instance_template"]
        .as_object_mut()
        .unwrap()
        .remove("allowed_local_range");
    admit(&s, A, parent).await;
    let mut child = fields();
    child["title"] = "Synthetic successor".into();
    child["routine_instance_template"]["after"] =
        json!([{"objective_id":A,"minimum_seconds":600,"maximum_seconds":1200}]);
    admit(&s, B, child).await;
    generate(&s).await;
    let result = put(&s, B, DAY, "2026-09-27T09:00:00Z", "2026-09-27T09:30:00Z").await;
    assert!(code(&result, "routine_override_violates_after_bounds"));
    generate(&s).await;
    assert_eq!(
        occurrence(&s, B, DAY).await["payload"]["static_window"]["start"],
        "2026-09-27T09:00:00Z"
    );
    println!("EVIDENCE[P1B35_test7]={}", result["diagnostics"]);
    let early = put(&s, B, DAY, "2026-09-27T07:35:00Z", "2026-09-27T08:05:00Z").await;
    assert!(code(&early, "routine_override_violates_after_bounds"));
    assert_eq!(
        occurrence(&s, B, DAY).await["payload"]["static_window"]["start"],
        "2026-09-27T07:35:00Z"
    );
}
#[tokio::test]
async fn exdates_and_nonoccurrence_dates_are_rejected_without_phantoms() {
    for exclusion in [
        json!({"exdates":[DAY]}),
        json!({"rule":{"kind":"weekly","weekdays":["mon"]}}),
        json!({"enabled_from":"2026-09-28"}),
    ] {
        let (s, _) = state().await;
        let mut f = fields();
        f["recurrence"]
            .as_object_mut()
            .unwrap()
            .extend(exclusion.as_object().unwrap().clone());
        admit(&s, A, f).await;
        let before = object(&s, A).await;
        let writes = counts(&s).await;
        let (status,result)=request(&s,"PUT",&path(A,DAY),json!({"schema_version":VERSION,"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(code(&result, "routine_override_no_occurrence"));
        assert_eq!(object(&s, A).await, before);
        assert_eq!(counts(&s).await, writes);
        let (status, deleted) = request(&s, "DELETE", &path(A, DAY), Value::Null).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(code(&deleted, "routine_override_no_occurrence"));
        if exclusion.get("exdates").is_some() {
            println!("EVIDENCE[P1B35_test8]={result}");
        }
        // Canonical data arriving outside triage is diagnosed by the date loop too.
        let mut d = definition(&s, A).await;
        d.schedule.overrides.push(serde_json::from_value(json!({"local_date":DAY,"start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"})).unwrap());
        let derived = instantiate(&[d], sec(NOW), sec(END));
        assert!(derived
            .diagnostics
            .iter()
            .any(|d| d.code == "routine_override_no_occurrence"));
        assert!(!derived.occurrences.iter().any(|o| o.local_date == DAY));
    }
}
#[tokio::test]
async fn override_sanity_guards_reject_without_writes_including_delete_restoration() {
    let (s, _) = setup().await;
    for (start, end, expected) in [
        (
            "2026-09-27T15:00:00Z",
            "2026-09-27T15:00:00Z",
            "routine_override_invalid_window",
        ),
        (
            "2026-09-27T15:30:00Z",
            "2026-09-27T15:00:00Z",
            "routine_override_invalid_window",
        ),
        (
            "2026-08-27T15:00:00Z",
            "2026-08-27T15:30:00Z",
            "routine_override_window_too_far",
        ),
        (
            "2026-10-27T15:00:00Z",
            "2026-10-27T15:30:00Z",
            "routine_override_window_too_far",
        ),
    ] {
        let before = object(&s, A).await;
        let task = occurrence(&s, A, DAY).await;
        let writes = counts(&s).await;
        let (status, result) = request(
            &s,
            "PUT",
            &path(A, DAY),
            json!({"schema_version":VERSION,"start":start,"end":end}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(code(&result, expected), "{result}");
        assert_eq!(object(&s, A).await, before);
        assert_eq!(occurrence(&s, A, DAY).await, task);
        assert_eq!(counts(&s).await, writes);
    }
    let (s, _) = state().await;
    let mut f = fields();
    f["routine_instance_template"]["placement"] = "static".into();
    f["routine_instance_template"]
        .as_object_mut()
        .unwrap()
        .remove("allowed_local_range");
    f["routine_instance_template"]["duration_estimate"]["seconds"] = json!(3 * 86400);
    admit(&s, A, f).await;
    pin(&s).await;
    let before = object(&s, A).await;
    let writes = counts(&s).await;
    let (status, result) = request(&s, "DELETE", &path(A, DAY), Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(code(&result, "routine_override_window_too_far"));
    assert_eq!(object(&s, A).await, before);
    assert_eq!(counts(&s).await, writes);
}
#[tokio::test]
async fn delete_removes_override_and_restores_nominal_window_with_same_key() {
    let (s, _) = setup().await;
    let before = occurrence(&s, A, DAY).await;
    pin(&s).await;
    let cleared = ok(&s, "DELETE", &path(A, DAY), Value::Null).await;
    assert_eq!(cleared["overridden"], false);
    let after = occurrence(&s, A, DAY).await;
    assert_eq!(before["id"], after["id"]);
    assert_eq!(
        before["payload"]["occurrence"],
        after["payload"]["occurrence"]
    );
    assert_eq!(
        before["payload"]["allowed_time_range"],
        after["payload"]["allowed_time_range"]
    );
    assert!(after["payload"].get("static_window").is_none());
    assert!(object(&s, A).await["payload"]["recurrence"]
        .get("overrides")
        .is_none());
    generate(&s).await;
    assert_eq!(occurrence(&s, A, DAY).await, after);
    let raw:String=sqlx::query_scalar("SELECT payload_json FROM logs WHERE json_extract(payload_json,'$.action')='clear_occurrence_override'").fetch_one(s.inner().store.pool()).await.unwrap();
    let log: Value = serde_json::from_str(&raw).unwrap();
    assert!(log.get("source").is_none());
}
#[tokio::test]
async fn ordinary_task_edits_still_reject_occurrences_before_and_after_override() {
    let (s, _) = setup().await;
    for pinned in [false, true] {
        if pinned {
            pin(&s).await;
        }
        let before = occurrence(&s, A, DAY).await;
        let id = before["id"].as_str().unwrap();
        let writes = counts(&s).await;
        let (status,result)=request(&s,"PATCH",&format!("/task/{id}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":before["version"],"static_window":{"start":"2026-09-27T16:00:00Z","end":"2026-09-27T16:30:00Z"},"title":"Forbidden direct edit"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(code(&result, "routine_occurrence_not_editable"));
        assert_eq!(object(&s, id).await, before);
        assert_eq!(counts(&s).await, writes);
    }
}
