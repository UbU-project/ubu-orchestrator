//! P1B-53 §E: the Calendar preview says what an event's placement is. It is read
//! from the Plan step or the Task record, and never inferred from the colour.
//! Synthetic and offline; every title and event id is invented.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{planning_time::FixedClock, services::calendar_client::RecordingCalendarApi, state::AppState};

// A Dynamic Task; a Static Task with a category; a Static Task with none.
const DYNAMIC: &str = A;
const STATIC_PERSONAL: &str = B;
const STATIC_BARE: &str = C;
const FOREIGN: &str = "5n0q8c9h7g4k2m1p3r6t8v0a2c";
const ROUTINE: &str = "Synthetic nightly teapot rest";

async fn setup() -> AppState {
    // One event on the calendar that UbU did not make, with a colour that maps to nothing here.
    let calendar = Arc::new(RecordingCalendarApi::with_wire_events(&json!({"items":[
        {"id":FOREIGN,"summary":"Synthetic teapot delivery","start":{"dateTime":"2026-09-29T15:00:00Z"},"end":{"dateTime":"2026-09-29T15:30:00Z"},
         "transparency":"opaque","reminders":{"useDefault":false,"overrides":[]}}
    ]})));
    let state = bare().await.with_calendar_api(calendar);
    seed(&state, DYNAMIC, "active", json!({"duration_estimate":{"type":"fixed","seconds":1800},"category_tag":"work","tags":["work"]})).await;
    seed(&state, STATIC_PERSONAL, "active", json!({"static_window":{"start":"2026-09-29T11:00:00Z","end":"2026-09-29T11:30:00Z"},"category_tag":"personal","tags":["personal"]})).await;
    seed(&state, STATIC_BARE, "active", json!({"static_window":{"start":"2026-09-29T13:00:00Z","end":"2026-09-29T13:30:00Z"}})).await;
    // A Static routine with no category: the shape of a night block.
    let (status, body) = request(&state, "POST", "/objective", json!({
        "schema_version":"ubu.orchestrator.objective.v1","mode":"evergreen","title":ROUTINE,
        "recurrence":{"timezone":"UTC","rule":{"kind":"daily"}},
        "routine_instance_template":{"title":ROUTINE,"duration_estimate":{"type":"fixed","seconds":28800},"nominal_start":"23:00:00",
            "placement":"static","occupies_capacity":true,"tags":[],"reminder_minutes":[]}
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    state
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert!(status.is_success(), "{method} {path}: {status} {body}");
    body
}
async fn generate(state: &AppState) -> Value {
    ok(state, "POST", "/planning/generate", json!({"schema_version":"planning-kernel-contract/0.1","request":null})).await
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
/// The one operation for a Task: (kind, static_anchor, colour).
fn operation<'a>(preview: &'a Value, task_id: &str) -> (&'a str, &'a Value, &'a Value) {
    let found: Vec<&Value> = preview["operations"].as_array().unwrap().iter().filter(|op| op["event"]["task_id"] == task_id).collect();
    assert_eq!(found.len(), 1, "one operation for {task_id}: {preview}");
    (found[0]["kind"].as_str().unwrap(), &found[0]["static_anchor"], &found[0]["event"]["color_id"])
}

#[tokio::test]
async fn create_and_update_carry_the_placement_from_the_task_and_never_from_the_colour() {
    let state = setup().await;
    // Capture claims the foreign event: a Static Task with no category, because its event has no colour.
    ok(&state, "POST", "/projection/calendar/capture", json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"})).await;
    let plan = generate(&state).await;
    let occurrence = plan["plan"]["steps"].as_array().unwrap().iter().find(|step| step["summary"] == ROUTINE).expect("the routine is in the Plan")["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let first = preview(&state).await;

    // ---- create
    // A planned Dynamic Task: not Static. It has a category and still no colour.
    assert_eq!(operation(&first, DYNAMIC), ("create", &json!(false), &Value::Null));
    // A Static Task with a category: Static, and coloured for it.
    assert_eq!(operation(&first, STATIC_PERSONAL), ("create", &json!(true), &json!("3")));
    // A Static Task with NO category has NO colour, and it is Static all the same.
    // Inferring placement from the colour called this one Dynamic.
    assert_eq!(operation(&first, STATIC_BARE), ("create", &json!(true), &Value::Null));
    // A routine occurrence, Static with no category: the night block.
    assert_eq!(operation(&first, &occurrence), ("create", &json!(true), &Value::Null));
    // The property is on every create, and a boolean every time.
    for op in first["operations"].as_array().unwrap() {
        assert_eq!(op["kind"], "create", "{op}");
        assert!(op["static_anchor"].is_boolean(), "{op}");
    }
    // Nothing is proposed for the captured event yet: capture recorded it as applied.
    let captured = ok(&state, "GET", "/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active", Value::Null).await["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["title"] == "Synthetic teapot delivery")
        .expect("the foreign event was captured")
        .clone();
    let captured_id = captured["task_id"].as_str().unwrap();
    assert!(first["operations"].as_array().unwrap().iter().all(|op| op["event"]["task_id"] != captured_id));
    ok(&state, "POST", "/projection/calendar/approve", json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":first["preview_id"],"authority_source":"user","export_mode":"mock"})).await;

    // ---- update
    // Move the captured Task and the bare Static Task in UbU, and re-plan five minutes on.
    for (id, version, start, end) in [(captured_id, captured["version"].as_i64().unwrap(), "2026-09-29T16:00:00Z", "2026-09-29T16:30:00Z"), (STATIC_BARE, 1, "2026-09-29T14:00:00Z", "2026-09-29T14:30:00Z")] {
        ok(&state, "PATCH", &format!("/task/{id}"), json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":version,"static_window":{"start":start,"end":end}})).await;
    }
    let later = state.clone().with_clock(FixedClock(UbuTimestamp::parse("2026-09-29T08:05:00Z").unwrap()));
    generate(&later).await;
    let second = preview(&later).await;
    // A captured foreign event: Static, and with no colour, because its event had none.
    assert_eq!(operation(&second, captured_id), ("update", &json!(true), &Value::Null));
    assert_eq!(operation(&second, STATIC_BARE), ("update", &json!(true), &Value::Null));
    // The Dynamic Task moved with the re-plan: an update, and still not Static.
    assert_eq!(operation(&second, DYNAMIC), ("update", &json!(false), &Value::Null));
    for op in second["operations"].as_array().unwrap() {
        assert_eq!(op["kind"], "update", "{op}");
        assert!(op["static_anchor"].is_boolean(), "{op}");
    }
    println!("P1B53_E_CREATE={}", json!(first["operations"].as_array().unwrap().iter().map(|op| json!([op["kind"], op["event"]["summary"], op["static_anchor"], op["event"]["color_id"]])).collect::<Vec<_>>()));
    println!("P1B53_E_UPDATE={}", json!(second["operations"].as_array().unwrap().iter().map(|op| json!([op["kind"], op["event"]["summary"], op["static_anchor"], op["event"]["color_id"]])).collect::<Vec<_>>()));
}

#[tokio::test]
async fn a_delete_carries_no_placement_and_the_event_body_is_unchanged() {
    let state = setup().await;
    generate(&state).await;
    let first = preview(&state).await;
    ok(&state, "POST", "/projection/calendar/approve", json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":first["preview_id"],"authority_source":"user","export_mode":"mock"})).await;
    // The placement is on the operation, not on the event: `events` is what it always was.
    for event in first["events"].as_array().unwrap() {
        assert!(event.get("static_anchor").is_none(), "{event}");
    }
    for op in first["operations"].as_array().unwrap() {
        assert!(op["event"].get("static_anchor").is_none(), "{op}");
    }
    // Retire the bare Static Task by clearing nothing and deleting its window's owner from the Plan:
    // a moot Task leaves the Plan, and its event is deleted.
    let row = task(&state, STATIC_BARE).await;
    let mut payload = row.clone();
    let object = payload.as_object_mut().unwrap();
    object.remove("__version");
    object.remove("__status");
    payload["status"] = "moot".into();
    payload["moot_reason_code"] = "user_declared_moot".into();
    let now = state.planning_now();
    let envelope = state
        .envelope_for([(ubu_core::UbuId::parse(STATIC_BARE).unwrap(), ubu_core::VersionRef::Version(1))].into_iter().collect(), ubu_core::AuthoritySource::User, now)
        .unwrap();
    ubu_store::queries::admit_object(state.inner().store.pool(), &envelope, ubu_store::models::object_record::NewObjectRecord {
        id: STATIC_BARE.into(), object_type: ubu_core::ObjectType::Task.as_str().into(), version: 2, status: "moot".into(),
        compartment_label: "synthetic-private-compartment".into(), payload, created_at: now.to_string(), updated_at: now.to_string(),
    }).await.unwrap();
    generate(&state).await;
    let second = preview(&state).await;
    let deletes: Vec<&Value> = second["operations"].as_array().unwrap().iter().filter(|op| op["kind"] == "delete").collect();
    assert_eq!(deletes.len(), 1, "{second}");
    assert_eq!(deletes[0]["external_id"], STATIC_BARE.strip_prefix("task_").unwrap());
    assert!(deletes[0].get("static_anchor").is_none() && deletes[0].get("event").is_none(), "{}", deletes[0]);
}
