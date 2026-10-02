//! P1B-55 §A: a colour decides the placement.
//!
//! An event with no colour is work for UbU to schedule: a Dynamic Task of the
//! event's length. An event with any colour is a commitment at its own time: a
//! Static Task, whose category is the colour's. It is the inverse of export, so a
//! round trip closes. Every title and id here is invented; every Calendar
//! operation uses a recorder.
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
const END: &str = "2026-09-25T20:00:00Z";
const ABSENT: &str = "Calendar event `{id}` has no colour, so it is taken as work for UbU to schedule: a Dynamic Task of the event's length, at no fixed time";

fn absent(id: &str) -> Value {
    json!({"code":"capture_colour_absent","message":ABSENT.replace("{id}", id)})
}
/// A synthetic event. `colour` is Google's colorId, or `None` for the default.
fn event(id: &str, title: &str, start: &str, end: &str, colour: Option<&str>) -> DesiredEvent {
    DesiredEvent {
        external_id: id.into(),
        task_id: format!("task_{id}"),
        summary: title.into(),
        start_at: format!("2026-09-25T{start}:00Z"),
        end_at: format!("2026-09-25T{end}:00Z"),
        color_id: colour.map(str::to_owned),
        transparent: false,
        reminders_minutes: vec![],
    }
}
async fn setup(events: Vec<DesiredEvent>) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::with_events(events));
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder.clone());
    (state, recorder)
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
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}
async fn capture(state: &AppState) -> Value {
    ok(state, "POST", "/projection/calendar/capture", json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"})).await
}
async fn generate(state: &AppState) -> Value {
    ok(state, "POST", "/planning/generate", json!({"horizon":{"start":NOW,"end":END}})).await
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, preview: &Value) -> Value {
    ok(state, "POST", "/projection/calendar/approve", json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"mock"})).await
}
fn counts(response: &Value) -> Value {
    json!({"captured":response["captured"],"updated":response["updated"],"unchanged":response["unchanged"],"skipped":response["skipped"]})
}
/// The stored Task that came from an event, with its row version.
async fn task(state: &AppState, source: &str) -> (i64, Value) {
    let (version, raw): (i64, String) = sqlx::query_as("SELECT version, payload_json FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.provenance.source.source_id')=?")
        .bind(source)
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    (version, serde_json::from_str(&raw).unwrap())
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
fn step<'a>(plan: &'a Value, task_id: &Value) -> &'a Value {
    plan["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| &step["task_id"] == task_id)
        .unwrap_or_else(|| panic!("no step for {task_id}: {plan}"))
}
fn codes(response: &Value) -> Vec<String> {
    response["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn an_uncoloured_event_becomes_a_dynamic_task_of_the_events_length() {
    let (state, recorder) = setup(vec![event("aaaaa", "Synthetic: descale the kettle", "09:00", "09:45", None)]).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":1,"updated":0,"unchanged":0,"skipped":0}));
    assert_eq!(response["diagnostics"], json!([absent("aaaaa")]));
    let (version, payload) = task(&state, "aaaaa").await;
    assert_eq!(version, 1);
    // The whole payload: a duration, and nothing that pins it or gives it a category.
    let id = payload["id"].clone();
    assert_eq!(
        payload,
        json!({
            "id": id, "status": "active", "title": "Synthetic: descale the kettle",
            "duration_estimate": {"type":"fixed","seconds":2700},
            "occupies_capacity": true,
            "provenance": {"created_at":NOW,"authority_source":"user","source":{"source_kind":"google_calendar","source_id":"aaaaa"}}
        })
    );
    for absent_field in ["static_window", "allowed_time_range", "category_tag", "tags"] {
        assert!(payload.get(absent_field).is_none(), "{absent_field}: {payload}");
    }
    println!("P1B55_A_DYNAMIC_PAYLOAD={payload}");
    // The handle is minted, and the event is recorded as it stands on the calendar.
    assert!(!id.as_str().unwrap().contains("aaaaa"));
    let applied = last_applied_events(state.inner().store.pool()).await.unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!((applied[0].external_id.as_str(), applied[0].task_id.as_str(), applied[0].start_at.as_str(), &applied[0].color_id), ("aaaaa", id.as_str().unwrap(), "2026-09-25T09:00:00Z", &None));
    assert_eq!(recorder.recorded_calls(), vec![RecordedCalendarCall::ListEvents]);
    // The Task list calls it planned, not static.
    let listed = ok(&state, "GET", "/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active", Value::Null).await;
    assert_eq!(listed["tasks"][0]["placement"], "planned");
}

#[tokio::test]
async fn a_coloured_event_is_captured_exactly_as_before() {
    let (state, _) = setup(vec![event("aaaaa", "Synthetic dentist", "09:00", "09:30", Some("3"))]).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":1,"updated":0,"unchanged":0,"skipped":0}));
    assert_eq!(response["diagnostics"], json!([]));
    let (_, payload) = task(&state, "aaaaa").await;
    let id = payload["id"].clone();
    // Field for field what capture stored before P1B-55.
    assert_eq!(
        payload,
        json!({
            "id": id, "status": "active", "title": "Synthetic dentist",
            "static_window": {"start":"2026-09-25T09:00:00Z","end":"2026-09-25T09:30:00Z"},
            "occupies_capacity": true, "category_tag": "personal", "tags": ["personal"],
            "provenance": {"created_at":NOW,"authority_source":"user","source":{"source_kind":"google_calendar","source_id":"aaaaa"}}
        })
    );
    assert!(payload.get("duration_estimate").is_none());
}

#[tokio::test]
async fn an_unmapped_or_ambiguous_colour_is_still_static_with_no_category() {
    let (state, _) = setup(vec![
        event("aaaaa", "Synthetic unmapped", "09:00", "09:30", Some("99")),
        event("bbbbb", "Synthetic ambiguous", "10:00", "10:30", Some("3")),
    ])
    .await;
    // Colour 3 is `personal` by default. A second category on it makes it ambiguous.
    ok(&state, "PUT", "/setting/calendar.color.work", json!({"schema_version":"ubu.orchestrator.setting.v1","value":"3"})).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":2,"updated":0,"unchanged":0,"skipped":0}));
    assert_eq!(
        response["diagnostics"],
        json!([
            {"code":"capture_colour_unmapped","message":"Calendar event `aaaaa` has unmapped colour `99`; no category assigned; map that colour in Settings to assign a category"},
            {"code":"capture_colour_ambiguous","message":"Calendar event `bbbbb` has a colour shared by multiple categories; no category assigned"}
        ])
    );
    for (source, start, end) in [("aaaaa", "09:00", "09:30"), ("bbbbb", "10:00", "10:30")] {
        let (_, payload) = task(&state, source).await;
        assert_eq!(payload["static_window"], json!({"start":format!("2026-09-25T{start}:00Z"),"end":format!("2026-09-25T{end}:00Z")}));
        assert!(payload.get("category_tag").is_none(), "{payload}");
        assert!(payload.get("duration_estimate").is_none(), "{payload}");
    }
}

#[tokio::test]
async fn a_zero_length_uncoloured_event_is_refused_and_admits_nothing() {
    let (state, _) = setup(vec![event("aaaaa", "Synthetic instant", "09:00", "09:00", None)]).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":0,"unchanged":0,"skipped":1}));
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"capture_event_invalid","message":"Calendar event has an unusable title or concrete time span; skipped"}])
    );
    assert_eq!(count(&state, "objects").await, 0);
    assert_eq!(count(&state, "projection_results").await, 0);
}

#[tokio::test]
async fn an_all_day_event_is_still_skipped_and_says_why() {
    let wire = json!({"items":[
        {"id":"aaaaa","summary":"Synthetic all day","start":{"date":"2026-09-25"},"end":{"date":"2026-09-26"}},
        {"id":"bbbbb","summary":"Synthetic: sort the button jar","start":{"dateTime":"2026-09-25T09:00:00Z"},"end":{"dateTime":"2026-09-25T09:30:00Z"},"reminders":{"useDefault":true}}
    ]});
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(&wire));
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder);
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":1,"updated":0,"unchanged":0,"skipped":1}));
    assert_eq!(
        response["diagnostics"],
        json!([
            {"code":"capture_all_day_unsupported","message":"list event `aaaaa` entry 0: all-day event has no dateTime; it carries no duration, so it cannot be scheduled and is skipped"},
            absent("bbbbb")
        ])
    );
    println!("P1B55_A_ALL_DAY={}", response["diagnostics"][0]["message"]);
    println!("P1B55_A_ABSENT={}", response["diagnostics"][1]["message"]);
    assert_eq!(count(&state, "objects").await, 1);
}

#[tokio::test]
async fn a_static_capture_that_loses_its_colour_becomes_dynamic_and_its_window_is_gone() {
    let coloured = event("aaaaa", "Synthetic dentist", "09:00", "09:45", Some("3"));
    let (state, recorder) = setup(vec![coloured.clone()]).await;
    capture(&state).await;
    let (version, before) = task(&state, "aaaaa").await;
    assert!(before["static_window"].is_object());
    println!("P1B55_A_FLIP_STATIC={before}");

    let mut uncoloured = coloured.clone();
    uncoloured.color_id = None;
    recorder.patch_event(&uncoloured).await.unwrap();
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":1,"unchanged":0,"skipped":0}));
    assert_eq!(response["diagnostics"], json!([absent("aaaaa")]));
    let (after_version, after) = task(&state, "aaaaa").await;
    assert_eq!(after_version, version + 1);
    // The same Task, by the same id. The window is gone, not left behind.
    assert_eq!(after["id"], before["id"]);
    assert!(after.get("static_window").is_none(), "{after}");
    assert_eq!(after["duration_estimate"], json!({"type":"fixed","seconds":2700}));
    assert!(after.get("category_tag").is_none(), "{after}");
    assert_eq!(after["occupies_capacity"], true);
    println!("P1B55_A_FLIP_DYNAMIC={after}");
    // One Task, and the applied record holds the event once, as it now stands.
    assert_eq!(count(&state, "objects").await, 1);
    let applied = last_applied_events(state.inner().store.pool()).await.unwrap();
    assert_eq!(applied.iter().map(|e| (e.external_id.as_str(), &e.color_id)).collect::<Vec<_>>(), vec![("aaaaa", &None)]);
    // And it is settled: the next capture changes nothing.
    assert_eq!(counts(&capture(&state).await), json!({"captured":0,"updated":0,"unchanged":1,"skipped":0}));
    // The planner now places it, somewhere of its own choosing.
    let plan = generate(&state).await;
    assert_eq!(step(&plan, &after["id"])["static_anchor"], false);
}

#[tokio::test]
async fn a_dynamic_capture_that_gains_a_colour_becomes_static_with_that_category() {
    let uncoloured = event("aaaaa", "Synthetic: descale the kettle", "09:00", "09:45", None);
    let (state, recorder) = setup(vec![uncoloured.clone()]).await;
    capture(&state).await;
    let (version, before) = task(&state, "aaaaa").await;
    assert!(before.get("static_window").is_none());

    let mut coloured = uncoloured.clone();
    coloured.color_id = Some("9".into());
    recorder.patch_event(&coloured).await.unwrap();
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":1,"unchanged":0,"skipped":0}));
    assert_eq!(response["diagnostics"], json!([]));
    let (after_version, after) = task(&state, "aaaaa").await;
    assert_eq!(after_version, version + 1);
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["static_window"], json!({"start":"2026-09-25T09:00:00Z","end":"2026-09-25T09:45:00Z"}));
    assert_eq!(after["category_tag"], "work");
    // One scheduling form: the duration went when the window came.
    assert!(after.get("duration_estimate").is_none(), "{after}");
    assert_eq!(after["status"], "active");
    // It is a commitment now, not a completion: no action was recorded against it.
    assert_eq!(count(&state, "logs").await, 0);
    let plan = generate(&state).await;
    let placed = step(&plan, &after["id"]);
    assert_eq!((placed["static_anchor"].clone(), placed["start_at"].clone()), (json!(true), json!("2026-09-25T09:00:00Z")));
    // A colour that maps to nothing makes it Static all the same, with no category.
    let other = event("bbbbb", "Synthetic: sort the button jar", "11:00", "11:20", None);
    let (state, recorder) = setup(vec![other.clone()]).await;
    capture(&state).await;
    let mut unmapped = other.clone();
    unmapped.color_id = Some("99".into());
    recorder.patch_event(&unmapped).await.unwrap();
    let response = capture(&state).await;
    assert_eq!(codes(&response), ["capture_colour_unmapped"]);
    let (_, after) = task(&state, "bbbbb").await;
    assert!(after["static_window"].is_object() && after.get("category_tag").is_none() && after.get("duration_estimate").is_none(), "{after}");
}

#[tokio::test]
async fn a_repeat_capture_of_an_uncoloured_event_admits_no_second_object() {
    let (state, _) = setup(vec![event("aaaaa", "Synthetic: descale the kettle", "09:00", "09:45", None)]).await;
    capture(&state).await;
    let (version, before) = task(&state, "aaaaa").await;
    let admissions = count(&state, "mutation_envelopes").await;
    let results = count(&state, "projection_results").await;
    let second = capture(&state).await;
    assert_eq!(counts(&second), json!({"captured":0,"updated":0,"unchanged":1,"skipped":0}));
    assert_eq!(second["diagnostics"], json!([]));
    assert_eq!(count(&state, "objects").await, 1);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions);
    assert_eq!(count(&state, "projection_results").await, results);
    assert_eq!(task(&state, "aaaaa").await, (version, before));
}

#[tokio::test]
async fn transparency_is_read_for_a_static_capture_and_not_for_a_dynamic_one() {
    let mut free_commitment = event("aaaaa", "Synthetic free meeting", "09:00", "09:30", Some("3"));
    free_commitment.transparent = true;
    let mut free_todo = event("bbbbb", "Synthetic free to-do", "10:00", "10:30", None);
    free_todo.transparent = true;
    let (state, _) = setup(vec![free_commitment, free_todo]).await;
    capture(&state).await;
    // A Free meeting that does not block is a real distinction, and is kept.
    assert_eq!(task(&state, "aaaaa").await.1["occupies_capacity"], false);
    // A Dynamic Task that occupied nothing would be scheduled into a void.
    let (_, todo) = task(&state, "bbbbb").await;
    assert_eq!(todo["occupies_capacity"], true);
    let plan = generate(&state).await;
    assert_eq!(step(&plan, &todo["id"])["occupies_capacity"], true);
}

#[tokio::test]
async fn two_overlapping_uncoloured_events_do_not_collide_and_both_are_placed() {
    // The same two events with a colour each are two commitments at one time, and collide.
    let overlapping = |colour: Option<&'static str>| vec![
        event("aaaaa", "Synthetic: descale the kettle", "09:00", "09:45", colour),
        event("bbbbb", "Synthetic: sort the button jar", "09:30", "10:00", colour),
    ];
    let (state, _) = setup(overlapping(Some("3"))).await;
    capture(&state).await;
    let collided = generate(&state).await;
    assert_eq!(codes(&collided).iter().filter(|code| *code == "static_task_collision").count(), 1, "{collided}");

    // Uncoloured, neither is Static, so there is nothing to collide.
    let (state, _) = setup(overlapping(None)).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":2,"updated":0,"unchanged":0,"skipped":0}));
    let plan = generate(&state).await;
    assert!(plan["plan"].is_object(), "{plan}");
    assert!(!codes(&plan).iter().any(|code| code == "static_task_collision" || code == "static_tasks_share_committed_time"), "{plan}");
    println!("P1B55_A_NO_COLLISION={}", json!({"diagnostics":plan["diagnostics"],"steps":plan["plan"]["steps"].as_array().unwrap().iter().map(|s| json!([s["start_at"], s["end_at"], s["static_anchor"], s["summary"]])).collect::<Vec<_>>()}));
    let steps = plan["plan"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|s| s["static_anchor"] == false));
    // Each keeps its own length, and they follow one another instead of overlapping.
    let mut windows: Vec<(u64, u64)> = steps.iter().map(|s| (s["start"].as_u64().unwrap(), s["end"].as_u64().unwrap())).collect();
    windows.sort();
    assert!(windows[0].1 <= windows[1].0, "{windows:?}");
    let mut lengths: Vec<u64> = windows.iter().map(|(start, end)| end - start).collect();
    lengths.sort();
    assert_eq!(lengths, [1800, 2700]);
    assert_eq!(plan["unplaced_tasks"], json!([]));
}

#[tokio::test]
async fn an_uncoloured_event_ubu_cannot_own_stays_a_commitment_at_its_own_time() {
    // An instance of a recurring event: UbU cannot write to it, so it cannot move it.
    const INSTANCE: &str = "0inv3nt3dc0unci1_20260925T090000Z";
    let (state, _) = setup(vec![event(INSTANCE, "Synthetic council", "09:00", "09:30", None)]).await;
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":1,"updated":0,"unchanged":0,"skipped":0}));
    assert_eq!(
        response["diagnostics"],
        json!([
            {"code":"capture_colour_absent","message":format!("Calendar event `{INSTANCE}` has no colour, but UbU cannot own it and so cannot move it: it stays a commitment at its own time, with no category")},
            {"code":"capture_occupancy_only","message":format!("Calendar event `{INSTANCE}` cannot be owned by UbU, so its time is recorded as an occupied window that UbU will never write back to or export")}
        ])
    );
    let (_, payload) = task(&state, INSTANCE).await;
    assert_eq!(payload["static_window"], json!({"start":"2026-09-25T09:00:00Z","end":"2026-09-25T09:30:00Z"}));
    assert!(payload.get("duration_estimate").is_none());
    assert_eq!(counts(&capture(&state).await), json!({"captured":0,"updated":0,"unchanged":1,"skipped":0}));
}

#[tokio::test]
async fn a_dynamic_capture_is_exported_as_an_update_to_its_new_window_with_no_colour_and_the_round_trip_closes() {
    let (state, recorder) = setup(vec![
        event("aaaaa", "Synthetic: descale the kettle", "15:00", "15:45", None),
        event("bbbbb", "Synthetic dentist", "09:00", "09:30", Some("3")),
    ])
    .await;
    capture(&state).await;
    let (_, todo) = task(&state, "aaaaa").await;
    let plan = generate(&state).await;
    let placed = step(&plan, &todo["id"]);
    // The planner chose the time: not the three o'clock the event was parked at.
    assert_ne!(placed["start_at"], "2026-09-25T15:00:00Z");
    let proposed = preview(&state).await;
    println!("P1B55_A_PREVIEW={}", proposed["operations"]);
    // One operation: the to-do moves to where it was planned, and carries no colour. The commitment is untouched.
    assert_eq!(
        proposed["operations"],
        json!([{"kind":"update","static_anchor":false,"event":{
            "external_id":"aaaaa","task_id":todo["id"],"summary":"Synthetic: descale the kettle",
            "start_at":placed["start_at"],"end_at":placed["end_at"],"color_id":null,"transparent":false,"reminders_minutes":[]
        }}])
    );
    recorder.clear_recorded_calls();
    assert_eq!(approve(&state, &proposed).await["status"], "applied");
    let moved = recorder.events().into_iter().find(|e| e.external_id == "aaaaa").unwrap();
    assert_eq!((moved.start_at.as_str(), &moved.color_id), (placed["start_at"].as_str().unwrap(), &None));
    // Captured again, colourless as UbU exported it: the same Dynamic Task, unchanged.
    let (version, before) = task(&state, "aaaaa").await;
    let again = capture(&state).await;
    assert_eq!(counts(&again), json!({"captured":0,"updated":0,"unchanged":2,"skipped":0}));
    assert_eq!(again["diagnostics"], json!([]));
    assert_eq!(task(&state, "aaaaa").await, (version, before));
    assert_eq!(preview(&state).await["operations"], json!([]));

    // Coloured after UbU moved it, it is a commitment at the time it now has. It is a
    // captured Task, so its colour is read by this rule and is not a completion.
    let mut pinned = moved.clone();
    pinned.color_id = Some("9".into());
    recorder.patch_event(&pinned).await.unwrap();
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":1,"unchanged":1,"skipped":0}));
    let (_, after) = task(&state, "aaaaa").await;
    assert_eq!(after["static_window"], json!({"start":placed["start_at"],"end":placed["end_at"]}));
    assert_eq!((after["category_tag"].clone(), after["status"].clone()), (json!("work"), json!("active")));
    let completions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type='task_done' OR (event_type='decision_recorded' AND json_extract(payload_json,'$.decision')='task_completed')")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(completions, 0);
}

#[tokio::test]
async fn a_dynamic_capture_that_does_not_fit_leaves_its_event_alone() {
    // Thirteen hours of work in a twelve-hour horizon: it cannot be placed.
    let (state, _) = setup(vec![event("aaaaa", "Synthetic: paint the whole imaginary fence", "08:30", "21:30", None)]).await;
    capture(&state).await;
    let (_, todo) = task(&state, "aaaaa").await;
    assert_eq!(todo["duration_estimate"], json!({"type":"fixed","seconds":13 * 3600}));
    let plan = generate(&state).await;
    assert_eq!(plan["unplaced_tasks"][0]["task_id"], todo["id"]);
    // Nothing is proposed against the event: it is not moved, and it is never deleted.
    assert_eq!(preview(&state).await["operations"], json!([]));
}

#[tokio::test]
async fn a_captured_commitment_recoloured_from_one_colour_to_another_is_still_drift() {
    // Unchanged by P1B-55, and on record: only gaining or losing a colour changes placement.
    let coloured = event("aaaaa", "Synthetic dentist", "09:00", "09:30", Some("3"));
    let (state, recorder) = setup(vec![coloured.clone()]).await;
    capture(&state).await;
    let before = task(&state, "aaaaa").await;
    let mut recoloured = coloured.clone();
    recoloured.color_id = Some("9".into());
    recorder.patch_event(&recoloured).await.unwrap();
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":0,"unchanged":0,"skipped":0}));
    assert_eq!(codes(&response), ["capture_owned_drift"]);
    assert_eq!(task(&state, "aaaaa").await, before);
}
