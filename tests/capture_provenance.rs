//! P1B-57 §C: capture recognises UbU's own stale exports.
//!
//! UbU stamps each event it creates with the Task it minted the event for. A store
//! that has neither an applied record for such an event nor that Task is looking at
//! UbU's own echo, from a store UbU no longer has. It becomes no Task and is reported
//! once. Every id and title here is invented; every Calendar operation uses a recorder.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc};
use tower::ServiceExt;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_capture::{plan_capture, stale_export_diagnostic, MAX_OCCUPANCY_NAMED},
        calendar_client::RecordingCalendarApi,
        calendar_wire::{parse_event_list, ubu_created_ids},
    },
    state::AppState,
};

const NOW: &str = "2026-09-25T08:00:00Z";
const END: &str = "2026-09-25T20:00:00Z";
// Synthetic, in the alphabet and length of an id UbU mints from a Task id's tail.
const ECHO: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
const OPERATORS: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e71";
const MISNAMED: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e72";
const STALE: &str = "capture_stale_export";

fn stale_one(id: &str) -> Value {
    json!({"code":STALE,"message":format!("Calendar event `{id}` was created by UbU for a Task this store does not have, so it is left alone and becomes no Task")})
}
/// A Google list item. `stamp` is the Task id the private property names, if any.
fn item(id: &str, title: &str, hour: &str, stamp: Option<&str>) -> Value {
    let mut item = json!({
        "id": id, "summary": title, "colorId": "3",
        "start": {"dateTime": format!("2026-09-25T{hour}:00:00Z")}, "end": {"dateTime": format!("2026-09-25T{hour}:30:00Z")},
        "reminders": {"useDefault": true}
    });
    if let Some(stamp) = stamp {
        item["extendedProperties"] = json!({"private": {"ubu_task": stamp}});
    }
    item
}
/// The three kinds of event: UbU's echo, the operator's with the same shape of id, and
/// one whose stamp names some other Task.
fn three_events() -> Value {
    json!({"items": [
        item(ECHO, "Synthetic leftover review", "09", Some(&format!("task_{ECHO}"))),
        item(OPERATORS, "Synthetic dentist", "10", None),
        item(MISNAMED, "Synthetic kettle descaling", "11", Some(&format!("task_{ECHO}"))),
    ]})
}
async fn state_with(recorder: Arc<RecordingCalendarApi>) -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder)
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
fn counts(response: &Value) -> Value {
    json!({"captured":response["captured"],"updated":response["updated"],"unchanged":response["unchanged"],"skipped":response["skipped"]})
}
async fn titles(state: &AppState) -> Vec<String> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT json_extract(payload_json,'$.title') FROM objects WHERE object_type='Task' ORDER BY 1")
        .fetch_all(state.inner().store.pool())
        .await
        .unwrap();
    rows
}

#[test]
fn a_stamped_event_whose_stamp_names_its_own_task_becomes_no_task() {
    let list = three_events();
    let (events, skipped) = parse_event_list(&list);
    assert!(skipped.is_empty());
    let created = ubu_created_ids(&list);
    assert_eq!(created, BTreeSet::from([ECHO.to_owned()]));
    let inverse = [("3".to_owned(), Some("personal".to_owned()))].into_iter().collect();

    let (tasks, diagnostics) = plan_capture(&events, &inverse, &Default::default(), &created);
    // The operator's event and the one with a stamp for some other Task are captured.
    assert_eq!(tasks.iter().map(|task| task.title.as_str()).collect::<Vec<_>>(), ["Synthetic dentist", "Synthetic kettle descaling"]);
    assert_eq!(serde_json::to_value(&diagnostics).unwrap(), json!([stale_one(ECHO)]));
    println!("P1B57_C_STALE={}", diagnostics[0].message);
    // Ids only: no title is echoed.
    assert!(!diagnostics[0].message.contains("Synthetic"));

    // Without the stamp set the same list is three Tasks, the echo included: as before P1B-57.
    let (tasks, diagnostics) = plan_capture(&events, &inverse, &Default::default(), &Default::default());
    assert_eq!(tasks.len(), 3);
    assert!(diagnostics.is_empty());
}

#[test]
fn the_stale_diagnostic_names_one_or_counts_several_and_is_absent_for_none() {
    assert!(stale_export_diagnostic(&[]).is_none());
    let one = stale_export_diagnostic(&[ECHO.to_owned()]).unwrap();
    assert_eq!(serde_json::to_value(&one).unwrap(), stale_one(ECHO));
    let ids: Vec<String> = (0..5).map(|n| format!("018f3c8e9b2a7c4d8f1e2a3b4c5d6e8{n}")).collect();
    assert_eq!(MAX_OCCUPANCY_NAMED, 3);
    let several = stale_export_diagnostic(&ids).unwrap();
    assert_eq!(several.code, STALE);
    assert_eq!(
        several.message,
        format!("5 Calendar events were created by UbU for Tasks this store does not have, so each is left alone and becomes no Task: `{}`, `{}`, `{}` and 2 more", ids[0], ids[1], ids[2])
    );
    let three = stale_export_diagnostic(&ids[..3]).unwrap();
    assert!(three.message.ends_with(&format!("`{}`, `{}`, `{}`", ids[0], ids[1], ids[2])), "{}", three.message);

    // Once for the whole capture, before the occupancy line, and after the per-event colour lines.
    let list = json!({"items": [
        item(&ids[0], "Synthetic a", "09", Some(&format!("task_{}", ids[0]))),
        item(&ids[1], "Synthetic b", "10", Some(&format!("task_{}", ids[1]))),
        {"id": "0inv3nt3dc0unci1_20260925T110000Z", "summary": "Synthetic council", "start": {"dateTime": "2026-09-25T11:00:00Z"}, "end": {"dateTime": "2026-09-25T11:30:00Z"}, "colorId": "99", "reminders": {"useDefault": true}},
    ]});
    let (events, _) = parse_event_list(&list);
    let (tasks, diagnostics) = plan_capture(&events, &Default::default(), &Default::default(), &ubu_created_ids(&list));
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        diagnostics.iter().map(|d| d.code.as_str()).collect::<Vec<_>>(),
        ["capture_colour_unmapped", STALE, "capture_occupancy_only"]
    );
    assert!(diagnostics[1].message.starts_with("2 Calendar events were created by UbU"));
}

#[test]
fn a_stale_echo_is_never_also_reported_as_invalid_and_a_task_the_store_holds_is_not_an_echo() {
    use ubu_orchestrator::services::calendar_projection::DesiredEvent;
    // An echo with no length: the stale check comes before the window is parsed.
    let instant = DesiredEvent {
        external_id: ECHO.into(), task_id: format!("task_{ECHO}"), summary: "Synthetic instant".into(),
        start_at: "2026-09-25T09:00:00Z".into(), end_at: "2026-09-25T09:00:00Z".into(),
        color_id: None, transparent: false, reminders_minutes: vec![],
    };
    let created = BTreeSet::from([ECHO.to_owned()]);
    let (tasks, diagnostics) = plan_capture(std::slice::from_ref(&instant), &Default::default(), &Default::default(), &created);
    assert!(tasks.is_empty());
    assert_eq!(serde_json::to_value(&diagnostics).unwrap(), json!([stale_one(ECHO)]));

    // The same id, when this store already holds a Task captured from it, is planned as that
    // Task: an event the store knows is not an echo of a store it no longer has.
    let mut known = instant.clone();
    known.end_at = "2026-09-25T09:30:00Z".into();
    let by_source = [(ECHO.to_owned(), "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e99".to_owned())].into_iter().collect();
    let (tasks, diagnostics) = plan_capture(&[known], &Default::default(), &by_source, &created);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id.as_deref(), Some("task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e99"));
    assert!(!diagnostics.iter().any(|d| d.code == STALE));
}

#[tokio::test]
async fn over_http_a_stamped_event_is_skipped_and_reported_and_the_unstamped_one_is_captured() {
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(&three_events()));
    let state = state_with(recorder.clone()).await;
    let response = capture(&state).await;
    // Two captured; the echo is a skip the operator sees counted.
    assert_eq!(counts(&response), json!({"captured":2,"updated":0,"unchanged":0,"skipped":1}));
    assert_eq!(response["diagnostics"], json!([stale_one(ECHO)]));
    assert_eq!(titles(&state).await, ["Synthetic dentist", "Synthetic kettle descaling"]);
    println!("P1B57_C_HTTP={response}");
    // The echo is not adopted: it is in no applied record, and nothing was written to the calendar.
    let applied = ubu_orchestrator::services::calendar_apply::last_applied_events(state.inner().store.pool()).await.unwrap();
    assert_eq!(applied.iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(), [OPERATORS, MISNAMED]);
    assert_eq!(recorder.events().len(), 3);

    // It is recognised again on every capture, and still becomes no Task.
    let again = capture(&state).await;
    assert_eq!(counts(&again), json!({"captured":0,"updated":0,"unchanged":2,"skipped":1}));
    assert_eq!(again["diagnostics"], json!([stale_one(ECHO)]));
    assert_eq!(titles(&state).await.len(), 2);

    // The Plan and the preview know nothing of it: it is left alone.
    ok(&state, "POST", "/planning/generate", json!({"horizon":{"start":NOW,"end":END}})).await;
    let preview = ok(&state, "GET", "/projection/calendar/preview", Value::Null).await;
    assert!(!preview.to_string().contains(ECHO), "{preview}");
}

#[tokio::test]
async fn the_stamp_changes_nothing_for_a_store_that_knows_the_task() {
    // A Task of this store's own, and its event on the calendar with UbU's stamp, but no applied
    // record: the database-reset case in which the Tasks were restored. That is `unrecorded`.
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = state_with(recorder.clone()).await;
    let task_id = ok(&state, "POST", "/task", json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Synthetic: sort the button jar","duration_estimate":{"type":"fixed","seconds":1800}})).await["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = task_id.strip_prefix("task_").unwrap().to_owned();
    let list = json!({"items": [item(&id, "Synthetic: sort the button jar", "09", Some(&task_id))]});
    assert_eq!(ubu_created_ids(&list), BTreeSet::from([id.clone()]));
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(&list));
    let state = state.with_calendar_api(recorder);

    let reconciled = ok(&state, "POST", "/projection/calendar/reconcile", json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"})).await;
    assert_eq!(reconciled["conflicts"][0]["conflict_type"], "unrecorded");
    let response = capture(&state).await;
    assert_eq!(counts(&response), json!({"captured":0,"updated":0,"unchanged":0,"skipped":1}));
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"capture_unrecorded_event","message":format!("Event `{id}` matches known Task evidence without applied ownership; not captured")}])
    );
    assert!(!response.to_string().contains(STALE));
    assert_eq!(titles(&state).await, ["Synthetic: sort the button jar"]);
}

#[tokio::test]
async fn an_event_one_store_exports_is_recognised_by_the_next_store_and_the_operators_is_captured() {
    // One calendar, two stores. The first exports a Task of its own; the calendar also holds an
    // event the operator made. This is a rehearsal's store being thrown away.
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(&json!({"items": [item(OPERATORS, "Synthetic dentist", "15", None)]})));
    let first = state_with(recorder.clone()).await;
    assert_eq!(counts(&capture(&first).await)["captured"], 1);
    ok(&first, "POST", "/task", json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Synthetic: sort the button jar","duration_estimate":{"type":"fixed","seconds":1800}})).await;
    ok(&first, "POST", "/planning/generate", json!({"horizon":{"start":NOW,"end":END}})).await;
    let preview = ok(&first, "GET", "/projection/calendar/preview", Value::Null).await;
    assert_eq!(preview["operations"].as_array().unwrap().iter().map(|op| op["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["create"]);
    let exported = preview["operations"][0]["event"]["external_id"].as_str().unwrap().to_owned();
    let approved = ok(&first, "POST", "/projection/calendar/approve", json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"mock"})).await;
    assert_eq!(approved["status"], "applied");
    // The store that exported it knows it: unchanged, and nothing is called stale.
    let same_store = capture(&first).await;
    assert_eq!(counts(&same_store), json!({"captured":0,"updated":0,"unchanged":2,"skipped":0}));
    assert_eq!(same_store["diagnostics"], json!([]));

    // A new store on the same calendar. Until P1B-57 it captured both events as the operator's.
    let second = state_with(recorder.clone()).await;
    let response = capture(&second).await;
    assert_eq!(counts(&response), json!({"captured":1,"updated":0,"unchanged":0,"skipped":1}));
    assert_eq!(response["diagnostics"], json!([stale_one(&exported)]));
    assert_eq!(titles(&second).await, ["Synthetic dentist"]);
    // The first store patched the operator's event when it captured nothing of the kind: his
    // event carries no stamp, and is his in every store.
    assert_eq!(recorder.events().len(), 2);
}
