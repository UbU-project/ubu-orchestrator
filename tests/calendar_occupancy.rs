//! P1B-51 §A: an event UbU cannot own is captured as occupied time it never
//! writes to. Synthetic wire fixtures, in-memory storage and a recording client
//! only; every title and every event id is invented.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_orchestrator::{
    services::{
        calendar_apply,
        calendar_capture::{occupancy_diagnostic, plan_capture, MAX_OCCUPANCY_NAMED},
        calendar_client::{RecordedCalendarCall, RecordingCalendarApi},
        calendar_projection::external_id,
        calendar_wire::parse_event,
    },
    state::AppState,
};

// An instance of a recurring event, in the shape Google gives one:
// {base32hex}_{timestamp}. The underscore and the uppercase letters are outside
// the alphabet UbU can own.
const RECURRING: &str = "0inv3nt3dc0unci1_20260929T090000Z";
const COUNCIL: &str = "Synthetic weekly teapot council";
// An ordinary id, which UbU can own.
const PLAIN: &str = "5n0q8c9h7g4k2m1p3r6t8v0a2c";
const DELIVERY: &str = "Synthetic one-off teapot delivery";
const WINDOW: (&str, &str) = ("2026-09-29T09:00:00Z", "2026-09-29T10:00:00Z");
const OCCUPANCY: &str = "capture_occupancy_only";

fn wire(id: &str, summary: &str, start: &str, end: &str) -> Value {
    json!({"id":id,"summary":summary,"start":{"dateTime":start},"end":{"dateTime":end},
        "colorId":"3","transparency":"opaque","reminders":{"useDefault":false,"overrides":[]}})
}
fn council() -> Value {
    wire(RECURRING, COUNCIL, WINDOW.0, WINDOW.1)
}
fn delivery() -> Value {
    wire(PLAIN, DELIVERY, "2026-09-29T12:00:00Z", "2026-09-29T12:30:00Z")
}
fn calendar(items: Vec<Value>) -> Arc<RecordingCalendarApi> {
    Arc::new(RecordingCalendarApi::with_wire_events(&json!({"items":items})))
}
async fn setup(items: Vec<Value>) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = calendar(items);
    (bare().await.with_calendar_api(recorder.clone()), recorder)
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
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
async fn generate(state: &AppState) -> Value {
    let planned = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"schema_version":"planning-kernel-contract/0.1","request":null}),
    )
    .await;
    assert!(planned["plan"].is_object(), "{planned}");
    planned
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn approve(state: &AppState, preview: &Value) -> Value {
    ok(state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"mock"})).await
}
async fn reconcile(state: &AppState) -> Value {
    ok(state,"POST","/projection/calendar/reconcile",json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"})).await
}
async fn active(state: &AppState) -> Vec<Value> {
    ok(
        state,
        "GET",
        "/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active",
        Value::Null,
    )
    .await["tasks"]
        .as_array()
        .cloned()
        .unwrap()
}
async fn titled(state: &AppState, title: &str) -> String {
    active(state)
        .await
        .iter()
        .find(|task| task["title"] == title)
        .unwrap_or_else(|| panic!("no active Task titled {title}"))["task_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
fn codes(response: &Value) -> Vec<(String, String)> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap().to_owned(), d["message"].as_str().unwrap().to_owned()))
        .collect()
}
/// Exactly the conflicts reconcile reported, as (kind, external id).
fn conflicts(response: &Value) -> Vec<(String, String)> {
    response["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["conflict_type"].as_str().unwrap().to_owned(), c["external_id"].as_str().unwrap().to_owned()))
        .collect()
}
fn foreign(id: &str) -> Vec<(String, String)> {
    vec![("foreign".to_owned(), id.to_owned())]
}
/// Three Dynamic Tasks of forty minutes each: two hours of work, which from
/// 08:00 would run straight through the council's 09:00 to 10:00 window.
async fn backlog(state: &AppState) {
    for id in [A, B, C] {
        seed(state, id, "active", json!({"duration_estimate":{"type":"fixed","seconds":2400}})).await;
    }
}
fn overlapping<'a>(plan: &'a Value, window: (&str, &str)) -> Vec<&'a Value> {
    plan["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["static_anchor"] == false)
        .filter(|step| step["start_at"].as_str().unwrap() < window.1 && step["end_at"].as_str().unwrap() > window.0)
        .collect()
}

// Properties 1 and 2: both are captured, only the unowned one says so, and both
// are Static Tasks with the colour's category.
#[tokio::test]
async fn a_recurring_instance_and_an_ownable_event_are_both_captured_and_only_the_first_is_occupancy_only() {
    assert!(external_id(&format!("task_{RECURRING}")).is_none(), "the fixture id must be one UbU cannot own");
    assert!(external_id(&format!("task_{PLAIN}")).is_some(), "the control id must be one UbU can own");
    let (state, recorder) = setup(vec![council(), delivery()]).await;
    let captured = capture(&state).await;
    assert_eq!(captured["captured"], 2, "{captured}");
    assert_eq!(captured["skipped"], 0, "{captured}");
    // One diagnostic in all: the occupancy notice, naming the id and never the title.
    assert_eq!(
        codes(&captured),
        vec![(OCCUPANCY.to_owned(), format!("Calendar event `{RECURRING}` cannot be owned by UbU, so its time is recorded as an occupied window that UbU will never write back to or export"))],
        "{captured}"
    );
    assert!(!captured["diagnostics"].to_string().contains(COUNCIL));
    assert!(!captured["diagnostics"].to_string().contains("teapot"));
    assert!(!captured["diagnostics"].to_string().contains(PLAIN));

    let tasks = active(&state).await;
    assert_eq!(tasks.len(), 2, "{tasks:?}");
    assert!(tasks.iter().all(|task| task["placement"] == "static"), "{tasks:?}");
    let unowned = task(&state, &titled(&state, COUNCIL).await).await;
    let owned = task(&state, &titled(&state, DELIVERY).await).await;
    for (payload, source, window) in [
        (&unowned, RECURRING, json!({"start":WINDOW.0,"end":WINDOW.1})),
        (&owned, PLAIN, json!({"start":"2026-09-29T12:00:00Z","end":"2026-09-29T12:30:00Z"})),
    ] {
        assert_eq!(payload["category_tag"], "personal", "{payload}");
        assert_eq!(payload["static_window"], window, "{payload}");
        assert_eq!(payload["occupies_capacity"], true, "{payload}");
        assert_eq!(
            payload["provenance"]["source"],
            json!({"source_kind":"google_calendar","source_id":source}),
            "{payload}"
        );
    }
    // The unowned Task's handle is minted: it is a handle UbU could own, and
    // nothing of the Google id is in it.
    let handle = unowned["id"].as_str().unwrap();
    assert!(external_id(handle).is_some(), "{handle}");
    assert!(!handle.contains("0inv3nt3dc0unci1"), "{handle}");
    assert!(recorder.recorded_calls().iter().all(|call| *call == RecordedCalendarCall::ListEvents));
    println!("P1B51_A_BOTH={}", json!({"captured":captured,"unowned":unowned["id"],"owned":owned["id"]}));
}

// Property 3: the export exclusion is specific. The unowned Task is planned and
// never offered to the calendar; the ownable one beside it still is.
#[tokio::test]
async fn the_preview_excludes_the_unowned_task_and_still_includes_the_ownable_one() {
    let (state, _) = setup(vec![council(), delivery()]).await;
    capture(&state).await;
    let unowned = titled(&state, COUNCIL).await;
    let owned = titled(&state, DELIVERY).await;
    let planned = generate(&state).await;
    let steps: Vec<&str> = planned["plan"]["steps"].as_array().unwrap().iter().map(|step| step["task_id"].as_str().unwrap()).collect();
    assert!(steps.contains(&unowned.as_str()), "the occupied window is in the Plan: {planned}");
    assert!(steps.contains(&owned.as_str()), "{planned}");

    let preview = preview(&state).await;
    let exported: Vec<&str> = preview["events"].as_array().unwrap().iter().map(|event| event["task_id"].as_str().unwrap()).collect();
    assert_eq!(exported, vec![owned.as_str()], "exactly the ownable Task is in the desired set: {preview}");
    assert!(!preview["events"].to_string().contains(RECURRING), "{preview}");
    assert!(!preview["operations"].to_string().contains(RECURRING), "{preview}");
    assert!(!preview["operations"].to_string().contains(&unowned), "{preview}");
    println!("P1B51_A_PREVIEW={}", json!({"events":preview["events"],"operations":preview["operations"],"diagnostics":preview["diagnostics"]}));
}

// Property 4: a repeat capture admits nothing and writes no envelope.
#[tokio::test]
async fn a_repeat_capture_admits_no_second_object_and_writes_no_second_envelope() {
    let (state, recorder) = setup(vec![council(), delivery()]).await;
    let first = capture(&state).await;
    let objects = count(&state, "objects").await;
    let envelopes = ledger(&state).await;
    let results = count(&state, "projection_results").await;
    let again = capture(&state).await;
    assert_eq!(count(&state, "objects").await, objects, "{again}");
    assert_eq!(task(&state, &titled(&state, COUNCIL).await).await["__version"], 1, "{again}");
    assert_eq!(ledger(&state).await, envelopes, "{again}");
    assert_eq!(count(&state, "projection_results").await, results, "{again}");
    assert_eq!(
        json!({"captured":again["captured"],"updated":again["updated"],"unchanged":again["unchanged"],"skipped":again["skipped"]}),
        json!({"captured":0,"updated":0,"unchanged":2,"skipped":0}),
        "{again}"
    );
    // It still says, every time, which event it does not own.
    assert_eq!(codes(&again), codes(&first));
    assert_eq!(active(&state).await.len(), 2);
    assert!(recorder.recorded_calls().iter().all(|call| *call == RecordedCalendarCall::ListEvents));
    println!("P1B51_A_REPEAT={}", json!({"first":first,"again":again,"objects":objects,"envelopes":envelopes}));
}

// Property 5, the one that matters most: the approve path. With nothing else to
// write, approving leaves the calendar byte for byte as it was, and the event is
// still foreign afterwards because UbU never recorded it as applied.
#[tokio::test]
async fn approving_after_an_unowned_capture_leaves_the_calendar_byte_for_byte_unchanged() {
    let (state, recorder) = setup(vec![council()]).await;
    let before = serde_json::to_string(&recorder.events()).unwrap();
    let foreign_before = reconcile(&state).await;
    assert_eq!(conflicts(&foreign_before), foreign(RECURRING));

    capture(&state).await;
    assert_eq!(count(&state, "projection_results").await, 0, "capture recorded nothing as applied");
    generate(&state).await;
    let proposed = preview(&state).await;
    assert_eq!(proposed["operations"], json!([]), "an unowned capture produced calendar operations");
    let approved = approve(&state, &proposed).await;
    assert_eq!(approved["status"], "applied", "{approved}");
    assert_eq!(approved["applied_events"], json!([]), "{approved}");

    assert_eq!(serde_json::to_string(&recorder.events()).unwrap(), before, "approving duplicated or altered the operator's own event");
    assert!(
        recorder.recorded_calls().iter().all(|call| *call == RecordedCalendarCall::ListEvents),
        "the calendar was written to: {:?}",
        recorder.recorded_calls()
    );
    assert!(calendar_apply::last_applied_events(state.inner().store.pool()).await.unwrap().is_empty());

    // Not vacuous: exactly one conflict, and it is this event, foreign, with the
    // same words as before the capture. Capturing occupancy is not ownership.
    let after = reconcile(&state).await;
    assert_eq!(conflicts(&after), foreign(RECURRING), "{after}");
    assert_eq!(after["status"], "observed", "{after}");
    assert_eq!(after["conflicts"], foreign_before["conflicts"]);
    assert_eq!(after["diagnostics"][0]["code"], "capture_event_not_ownable", "{after}");
    println!("P1B51_A_APPROVE={}", json!({"events_before":before,"calls":format!("{:?}", recorder.recorded_calls()),"reconcile_after":after["conflicts"]}));
}

// The same boundary when the approve does write: UbU's own work is created
// beside the commitment, and no call of any kind names the unowned event.
#[tokio::test]
async fn an_approve_that_writes_ubus_own_work_never_touches_the_unowned_event() {
    let (state, recorder) = setup(vec![council(), delivery()]).await;
    let untouched = serde_json::to_string(&recorder.events().into_iter().find(|event| event.external_id == RECURRING).unwrap()).unwrap();
    capture(&state).await;
    let unowned = titled(&state, COUNCIL).await;
    backlog(&state).await;
    generate(&state).await;
    let proposed = preview(&state).await;
    let kinds: Vec<&str> = proposed["operations"].as_array().unwrap().iter().map(|operation| operation["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["create", "create", "create"], "three creates for the three Dynamic Tasks and nothing else: {proposed}");
    let approved = approve(&state, &proposed).await;
    assert_eq!(approved["status"], "applied", "{approved}");

    let writes: Vec<RecordedCalendarCall> = recorder.recorded_calls().into_iter().filter(|call| *call != RecordedCalendarCall::ListEvents).collect();
    assert_eq!(writes.len(), 3, "{writes:?}");
    for call in &writes {
        match call {
            RecordedCalendarCall::InsertEvent { event } => {
                assert_ne!(event.external_id, RECURRING);
                assert_ne!(event.task_id, unowned);
                assert!([A, B, C].contains(&event.task_id.as_str()), "{event:?}");
            }
            other => panic!("the only writes are inserts of UbU's own work: {other:?}"),
        }
    }
    // The commitment itself is byte for byte what it was, and there is one of it.
    let now: Vec<_> = recorder.events().into_iter().filter(|event| event.summary == COUNCIL).collect();
    assert_eq!(now.len(), 1, "{now:?}");
    assert_eq!(serde_json::to_string(&now[0]).unwrap(), untouched);
    assert!(!approved["applied_events"].to_string().contains(RECURRING), "{approved}");
    // Reconcile: the unowned event is the only conflict, and nothing drifted.
    let after = reconcile(&state).await;
    assert_eq!(conflicts(&after), foreign(RECURRING), "{after}");
    println!("P1B51_A_WRITES={}", json!({"writes":format!("{writes:?}"),"reconcile":after["conflicts"]}));
}

// The reason for the whole change: the unowned Task occupies capacity, so the
// planner places no Dynamic work over its window.
#[tokio::test]
async fn the_planner_places_no_dynamic_work_over_an_unowned_window() {
    // The control: the same backlog with no commitment runs through 09:00 to
    // 10:00, so the window below is one the planner would otherwise use.
    let (free, _) = setup(vec![]).await;
    backlog(&free).await;
    let unconstrained = generate(&free).await;
    assert!(
        !overlapping(&unconstrained, WINDOW).is_empty(),
        "the control must place work inside the window, or the assertion below proves nothing: {unconstrained}"
    );

    let (state, _) = setup(vec![council()]).await;
    capture(&state).await;
    let unowned = titled(&state, COUNCIL).await;
    backlog(&state).await;
    let planned = generate(&state).await;
    let steps = planned["plan"]["steps"].as_array().unwrap();
    let anchor = steps.iter().find(|step| step["task_id"] == unowned.as_str()).expect("the occupied window is a step");
    assert_eq!(
        json!({"static_anchor":anchor["static_anchor"],"start_at":anchor["start_at"],"end_at":anchor["end_at"],"occupies_capacity":anchor["occupies_capacity"]}),
        json!({"static_anchor":true,"start_at":WINDOW.0,"end_at":WINDOW.1,"occupies_capacity":true})
    );
    // All three Dynamic Tasks are still placed, and none of them over the window.
    let dynamic: Vec<&Value> = steps.iter().filter(|step| step["static_anchor"] == false).collect();
    assert_eq!(dynamic.len(), 3, "{planned}");
    assert_eq!(planned["unplaced_tasks"], json!([]), "{planned}");
    assert!(overlapping(&planned, WINDOW).is_empty(), "Dynamic work was placed over the unowned window: {:?}", overlapping(&planned, WINDOW));
    println!(
        "P1B51_A_NO_OVERLAP={}",
        json!({
            "without_the_commitment": unconstrained["plan"]["steps"].as_array().unwrap().iter().map(|s| json!([s["start_at"], s["end_at"]])).collect::<Vec<_>>(),
            "with_it": steps.iter().map(|s| json!([s["start_at"], s["end_at"], s["static_anchor"]])).collect::<Vec<_>>()
        })
    );
}

// The calendar is the truth for an event UbU does not own: when the commitment
// moves there, the occupied window follows on the next capture, in place.
#[tokio::test]
async fn an_unowned_event_moved_in_the_calendar_moves_the_occupied_window_in_place() {
    let (state, _) = setup(vec![council()]).await;
    capture(&state).await;
    let unowned = titled(&state, COUNCIL).await;
    let objects = count(&state, "objects").await;
    let moved = state.clone().with_calendar_api(calendar(vec![wire(RECURRING, COUNCIL, "2026-09-29T14:00:00Z", "2026-09-29T15:00:00Z")]));
    let again = capture(&moved).await;
    assert_eq!(
        json!({"captured":again["captured"],"updated":again["updated"],"unchanged":again["unchanged"]}),
        json!({"captured":0,"updated":1,"unchanged":0}),
        "{again}"
    );
    assert_eq!(count(&moved, "objects").await, objects);
    let payload = task(&moved, &unowned).await;
    assert_eq!(payload["static_window"], json!({"start":"2026-09-29T14:00:00Z","end":"2026-09-29T15:00:00Z"}));
    assert_eq!(payload["__version"], 2);
    assert_eq!(count(&moved, "projection_results").await, 0, "a move is still not ownership");
}

// P1B-52 §D: one diagnostic per capture for every event UbU cannot own. A week
// of one daily commitment is one line naming the count, not seven lines.
const ONE: &str = "Calendar event `0inv3nt3dc0unci1_20260929T090000Z` cannot be owned by UbU, so its time is recorded as an occupied window that UbU will never write back to or export";
fn daily(days: u32) -> Vec<Value> {
    (0..days)
        .map(|day| {
            let date = 29 + day;
            // 29 and 30 September, then October: an invented week of one daily commitment.
            let (month, dom) = if date <= 30 { (9, date) } else { (10, date - 30) };
            wire(
                &format!("0inv3nt3dc0unci1_2026{month:02}{dom:02}T090000Z"),
                COUNCIL,
                &format!("2026-{month:02}-{dom:02}T09:00:00Z"),
                &format!("2026-{month:02}-{dom:02}T10:00:00Z"),
            )
        })
        .collect()
}
fn planned(items: Vec<Value>) -> (usize, Vec<(String, String)>) {
    let events: Vec<_> = items.iter().map(|item| parse_event(item).unwrap()).collect();
    let inverse = [("3".to_owned(), Some("personal".to_owned()))].into_iter().collect();
    let (tasks, diagnostics) = plan_capture(&events, &inverse, &Default::default());
    (tasks.len(), diagnostics.into_iter().map(|d| (d.code, d.message)).collect())
}

#[test]
fn a_week_of_one_daily_commitment_is_one_diagnostic_naming_the_count_and_the_first_few_ids() {
    let (tasks, diagnostics) = planned(daily(7));
    // Seven occupied windows, and one line about them.
    assert_eq!(tasks, 7);
    assert_eq!(
        diagnostics,
        vec![(
            OCCUPANCY.to_owned(),
            "7 Calendar events cannot be owned by UbU, so the time of each is recorded as an occupied window that UbU will never write back to or export: `0inv3nt3dc0unci1_20260929T090000Z`, `0inv3nt3dc0unci1_20260930T090000Z`, `0inv3nt3dc0unci1_20261001T090000Z` and 4 more".to_owned()
        )]
    );
    assert_eq!(MAX_OCCUPANCY_NAMED, 3);
    assert!(!diagnostics[0].1.contains("teapot"), "no title is echoed");
    println!("P1B52_D_COLLAPSED={}", diagnostics[0].1);
}

#[test]
fn a_single_instance_still_names_its_id_and_a_few_are_all_named() {
    // One: the sentence it always was.
    assert_eq!(planned(daily(1)).1, vec![(OCCUPANCY.to_owned(), ONE.to_owned())]);
    // Two and three: every id, and nothing counted.
    for days in [2, 3] {
        let (tasks, diagnostics) = planned(daily(days));
        assert_eq!(tasks, days as usize);
        assert_eq!(diagnostics.len(), 1);
        let message = &diagnostics[0].1;
        assert!(message.starts_with(&format!("{days} Calendar events cannot be owned by UbU")), "{message}");
        assert_eq!(message.matches("0inv3nt3dc0unci1_").count(), days as usize, "{message}");
        assert!(!message.contains("more"), "{message}");
    }
    // Four: three named, one counted.
    let four = &planned(daily(4)).1[0].1;
    assert!(four.ends_with("`0inv3nt3dc0unci1_20261001T090000Z` and 1 more"), "{four}");
    // None: nothing to say.
    assert!(occupancy_diagnostic(&[]).is_none());
    // An ownable event beside them is not counted and not named, and its colour diagnostic comes first.
    let mut items = daily(2);
    let mut uncoloured = delivery();
    uncoloured.as_object_mut().unwrap().remove("colorId");
    items.insert(1, uncoloured);
    let (tasks, diagnostics) = planned(items);
    assert_eq!(tasks, 3);
    assert_eq!(diagnostics.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(), ["capture_colour_absent", OCCUPANCY]);
    assert!(diagnostics[1].1.starts_with("2 Calendar events"), "{}", diagnostics[1].1);
    assert!(!diagnostics[1].1.contains(PLAIN), "{}", diagnostics[1].1);
}

// Through the route, for one capture of several instances inside the horizon.
#[tokio::test]
async fn one_capture_of_several_instances_reports_them_once_and_repeats_it_once() {
    let instances: Vec<Value> = ["09", "11", "13"]
        .iter()
        .map(|hour| wire(&format!("0inv3nt3dc0unci1_20260929T{hour}0000Z"), COUNCIL, &format!("2026-09-29T{hour}:00:00Z"), &format!("2026-09-29T{hour}:30:00Z")))
        .collect();
    let (state, _) = setup(instances).await;
    let captured = capture(&state).await;
    assert_eq!(captured["captured"], 3, "{captured}");
    let expected = vec![(
        OCCUPANCY.to_owned(),
        "3 Calendar events cannot be owned by UbU, so the time of each is recorded as an occupied window that UbU will never write back to or export: `0inv3nt3dc0unci1_20260929T090000Z`, `0inv3nt3dc0unci1_20260929T110000Z`, `0inv3nt3dc0unci1_20260929T130000Z`".to_owned(),
    )];
    assert_eq!(codes(&captured), expected, "{captured}");
    // Each is still its own Static Task. Nothing knows they are the same commitment.
    let tasks = active(&state).await;
    assert_eq!(tasks.len(), 3);
    assert!(tasks.iter().all(|task| task["placement"] == "static" && task["title"] == COUNCIL));
    let again = capture(&state).await;
    assert_eq!(again["unchanged"], 3, "{again}");
    assert_eq!(codes(&again), expected, "{again}");
}
