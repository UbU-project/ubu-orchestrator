//! P1B-53 §C: a completed Task's calendar event is frozen. Synthetic and offline:
//! in-memory storage and a recording client; every title is invented.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    planning_time::FixedClock,
    services::{
        calendar_apply::{retained_diagnostic, MAX_RETAINED_NAMED},
        calendar_client::{RecordedCalendarCall, RecordingCalendarApi},
    },
    state::AppState,
};

const D: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e73";
const E: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e74";
const ACTION: &str = "ubu.orchestrator.task_action.v1";
const RETAINED: &str = "calendar_event_retained";
/// Ten minutes after the fixture's clock: a re-plan here moves every Dynamic window.
const LATER: &str = "2026-09-29T08:10:00Z";

fn ext(task_id: &str) -> &str {
    task_id.strip_prefix("task_").unwrap()
}
async fn setup(ids: &[&str]) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = bare().await.with_calendar_api(recorder.clone());
    for id in ids {
        seed(&state, id, "active", json!({"duration_estimate":{"type":"fixed","seconds":1800}})).await;
    }
    (state, recorder)
}
fn later(state: &AppState) -> AppState {
    state.clone().with_clock(FixedClock(UbuTimestamp::parse(LATER).unwrap()))
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    body
}
async fn generate(state: &AppState) -> Value {
    ok(state, "POST", "/planning/generate", json!({"schema_version":"planning-kernel-contract/0.1","request":null})).await
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, preview: &Value) -> Value {
    ok(state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"mock"})).await
}
/// Generate, preview and approve: one pass of the loop.
async fn apply(state: &AppState) -> (Value, Value) {
    generate(state).await;
    let proposed = preview(state).await;
    let approved = approve(state, &proposed).await;
    assert_eq!(approved["status"], "applied", "{approved}");
    (proposed, approved)
}
async fn complete(state: &AppState, id: &str) -> String {
    ok(state, "POST", &format!("/task/{id}/action"), json!({"schema_version":ACTION,"action":"complete"})).await["log_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
/// A Task that did not happen: straight to the store, as the routine machinery writes it.
async fn set_status(state: &AppState, id: &str, status: &str) {
    let mut payload = task(state, id).await;
    let version = payload["__version"].as_i64().unwrap();
    let object = payload.as_object_mut().unwrap();
    object.remove("__version");
    object.remove("__status");
    payload["status"] = status.into();
    if status == "moot" {
        payload["moot_reason_code"] = "user_declared_moot".into();
    }
    let now = state.planning_now();
    let envelope = state
        .envelope_for([(UbuId::parse(id).unwrap(), VersionRef::Version(version as u64))].into_iter().collect(), AuthoritySource::User, now)
        .unwrap();
    ubu_store::queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::object_record::NewObjectRecord {
            id: id.into(),
            object_type: ObjectType::Task.as_str().into(),
            version: version + 1,
            status: status.into(),
            compartment_label: "synthetic-private-compartment".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
/// Every operation of a preview as (kind, external id).
fn operations(preview: &Value) -> Vec<(String, String)> {
    preview["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| {
            let id = op["event"]["external_id"].as_str().or(op["external_id"].as_str()).unwrap();
            (op["kind"].as_str().unwrap().to_owned(), id.to_owned())
        })
        .collect()
}
fn retained(preview: &Value) -> Vec<String> {
    preview["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == RETAINED)
        .map(|d| d["message"].as_str().unwrap().to_owned())
        .collect()
}
fn writes_to(recorder: &RecordingCalendarApi, external_id: &str) -> Vec<String> {
    recorder
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            RecordedCalendarCall::InsertEvent { event } if event.external_id == external_id => Some("insert".to_owned()),
            RecordedCalendarCall::PatchEvent { event } if event.external_id == external_id => Some("patch".to_owned()),
            RecordedCalendarCall::DeleteEvent { external_id: id } if id == external_id => Some("delete".to_owned()),
            _ => None,
        })
        .collect()
}
fn event_of(recorder: &RecordingCalendarApi, external_id: &str) -> Option<String> {
    recorder.events().into_iter().find(|event| event.external_id == external_id).map(|event| serde_json::to_string(&event).unwrap())
}

#[tokio::test]
async fn a_completed_tasks_event_is_neither_updated_nor_deleted_and_a_reopened_one_is_managed_again() {
    let (state, recorder) = setup(&[A, B]).await;
    apply(&state).await;
    let as_applied = event_of(&recorder, ext(A)).expect("A's event was created");
    assert_eq!(writes_to(&recorder, ext(A)), ["insert"]);

    let completion = complete(&state, A).await;
    // Ten minutes on, the Plan no longer holds A and every other window has moved.
    let state = later(&state);
    generate(&state).await;
    let proposed = preview(&state).await;
    // B moves, so the re-plan is real. A takes no operation of any kind.
    assert_eq!(operations(&proposed), vec![("update".to_owned(), ext(B).to_owned())], "{proposed}");
    assert_eq!(
        retained(&proposed),
        vec![format!("Calendar event `{}` is left as it is: it is the record of a completed Task, and is neither updated nor deleted", ext(A))]
    );
    let approved = approve(&state, &proposed).await;
    assert_eq!(approved["status"], "applied");
    // On the calendar: still there, and byte for byte what it was. No patch, no delete.
    assert_eq!(event_of(&recorder, ext(A)).as_deref(), Some(as_applied.as_str()));
    assert_eq!(writes_to(&recorder, ext(A)), ["insert"]);
    // In UbU's record of what it applied: still there.
    assert!(approved["applied_events"].as_array().unwrap().iter().any(|event| event["external_id"] == ext(A)), "{approved}");
    // And again on the next pass: frozen means every time, not once.
    let (again, _) = apply(&state).await;
    assert!(operations(&again).iter().all(|(_, id)| id != ext(A)), "{again}");
    assert_eq!(retained(&again).len(), 1);
    assert_eq!(writes_to(&recorder, ext(A)), ["insert"]);

    // Reopened, it is active, and it is managed again: the same event is updated to its new window.
    let (status, undone) = request(&state, "POST", &format!("/task/{A}/reopen"), json!({"schema_version":ACTION,"completion_log_id":completion})).await;
    assert_eq!(status, StatusCode::OK, "{undone}");
    generate(&state).await;
    let managed = preview(&state).await;
    assert!(operations(&managed).contains(&("update".to_owned(), ext(A).to_owned())), "{managed}");
    assert_eq!(retained(&managed), Vec::<String>::new(), "{managed}");
    approve(&state, &managed).await;
    assert_eq!(writes_to(&recorder, ext(A)), ["insert", "patch"]);
    assert_ne!(event_of(&recorder, ext(A)).unwrap(), as_applied);
}

#[tokio::test]
async fn a_failed_tasks_event_and_a_moot_tasks_event_are_still_deleted() {
    let (state, recorder) = setup(&[A, B, C]).await;
    apply(&state).await;
    // Neither happened. Only `completed` freezes an event.
    set_status(&state, A, "failed").await;
    set_status(&state, B, "moot").await;
    let state = later(&state);
    generate(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(
        operations(&proposed),
        vec![
            ("update".to_owned(), ext(C).to_owned()),
            ("delete".to_owned(), ext(A).to_owned()),
            ("delete".to_owned(), ext(B).to_owned())
        ],
        "{proposed}"
    );
    assert_eq!(retained(&proposed), Vec::<String>::new());
    approve(&state, &proposed).await;
    assert_eq!(writes_to(&recorder, ext(A)), ["insert", "delete"]);
    assert_eq!(writes_to(&recorder, ext(B)), ["insert", "delete"]);
    assert_eq!(event_of(&recorder, ext(A)), None);
    assert_eq!(event_of(&recorder, ext(B)), None);
}

#[tokio::test]
async fn the_retained_diagnostic_names_the_count_and_collapses_past_a_few() {
    let (state, recorder) = setup(&[A, B, C, D, E]).await;
    apply(&state).await;
    assert_eq!(retained(&preview(&state).await), Vec::<String>::new(), "nothing completed, nothing to say");
    // Two: both named, nothing counted.
    for id in [A, B] {
        complete(&state, id).await;
    }
    let two = retained(&preview(&later(&state)).await);
    assert_eq!(
        two,
        vec![format!("2 Calendar events are left as they are: each is the record of a completed Task, and is neither updated nor deleted: `{}`, `{}`", ext(A), ext(B))]
    );
    // Five: one diagnostic, the first three named, two counted.
    for id in [C, D, E] {
        complete(&state, id).await;
    }
    let state = later(&state);
    generate(&state).await;
    let proposed = preview(&state).await;
    let five = retained(&proposed);
    assert_eq!(
        five,
        vec![format!(
            "5 Calendar events are left as they are: each is the record of a completed Task, and is neither updated nor deleted: `{}`, `{}`, `{}` and 2 more",
            ext(A), ext(B), ext(C)
        )]
    );
    assert_eq!(MAX_RETAINED_NAMED, 3);
    assert!(!five[0].contains("teapot"), "ids only, never a title");
    // Every Task is done and out of the Plan, and not one event is touched.
    assert_eq!(operations(&proposed), vec![], "{proposed}");
    approve(&state, &proposed).await;
    assert_eq!(recorder.events().len(), 5);
    assert!(recorder.recorded_calls().iter().all(|call| !matches!(call, RecordedCalendarCall::PatchEvent { .. } | RecordedCalendarCall::DeleteEvent { .. })));
    // The function itself: nothing retained says nothing.
    assert!(retained_diagnostic(&Default::default()).is_none());
    println!("P1B53_C_RETAINED={}", five[0]);
}

// P1B-60: route-level requests are in-process, with the recording Calendar only.
#[tokio::test]
async fn matching_placements_counts_all_current_matches_without_operations() {
    let (state, _) = setup(&[A, B]).await;
    apply(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(proposed["matching_placements"], 2);
    assert!(operations(&proposed).is_empty());
}

#[tokio::test]
async fn matching_placements_is_zero_when_every_placement_moves() {
    let (state, _) = setup(&[A, B]).await;
    apply(&state).await;
    let state = later(&state);
    generate(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(proposed["matching_placements"], 0);
    assert_eq!(operations(&proposed), vec![("update".into(), ext(A).into()), ("update".into(), ext(B).into())]);
}

#[tokio::test]
async fn matching_placements_excludes_retained_calendar_completed_history() {
    let (state, recorder) = setup(&[A, B]).await;
    apply(&state).await;
    let mut observed = recorder.events().into_iter().find(|event| event.task_id == A).unwrap();
    observed.color_id = Some("10".into());
    recorder.place_event(observed);
    let captured = ok(&state, "POST", "/projection/calendar/capture", json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"})).await;
    assert_eq!(captured["updated"], 1);
    assert_eq!(task(&state, A).await["status"], "completed");
    // Regenerate and apply B's new placement. A remains only as history.
    apply(&state).await;
    let proposed = preview(&state).await;
    assert!(proposed["events"].as_array().unwrap().iter().any(|event| event["task_id"] == A));
    assert_eq!(retained(&proposed).len(), 1);
    assert!(operations(&proposed).is_empty());
    assert_eq!(proposed["matching_placements"], 1);
    println!("P1B60_RETAINED={}", json!({"desired_events":proposed["events"].as_array().unwrap().len(),"retained":retained(&proposed).len(),"matching_placements":proposed["matching_placements"],"operations":proposed["operations"]}));
}

async fn pin_for_matching_test(state: &AppState, id: &str, start: &str, end: &str) {
    ok(state, "PATCH", &format!("/task/{id}"), json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"static_window":{"start":start,"end":end}})).await;
}

#[tokio::test]
async fn matching_count_is_zero_for_static_only_plan() {
    let (state, _) = setup(&[A, B]).await;
    pin_for_matching_test(&state, A, "2026-09-29T10:00:00Z", "2026-09-29T10:30:00Z").await;
    pin_for_matching_test(&state, B, "2026-09-29T11:00:00Z", "2026-09-29T11:30:00Z").await;
    apply(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(proposed["events"].as_array().unwrap().len(), 2);
    assert!(operations(&proposed).is_empty());
    assert_eq!(proposed["matching_placements"], 0);
}

#[tokio::test]
async fn matching_count_in_a_mixed_plan_counts_only_dynamic_work() {
    let (state, _) = setup(&[A, B]).await;
    pin_for_matching_test(&state, A, "2026-09-29T10:00:00Z", "2026-09-29T10:30:00Z").await;
    apply(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(proposed["events"].as_array().unwrap().len(), 2);
    assert!(operations(&proposed).is_empty());
    assert_eq!(proposed["matching_placements"], 1);
}
