//! P1B-44: synthetic wire fixtures, in-memory storage, and a recording client only.
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_apply, calendar_capture,
        calendar_client::{CalendarExportMode, RecordedCalendarCall, RecordingCalendarApi},
        calendar_reconciliation_service,
        calendar_wire::{list_diagnostic, parse_event, parse_event_list},
    },
    state::AppState,
};

const ID: &str = "abc123def456ghij_20260928T163000Z";
const TITLE: &str = "Synthetic lunar teapot rehearsal";
fn fixture(id: &str) -> Value {
    json!({"id":id,"summary":TITLE,"start":{"dateTime":"2026-09-28T16:30:00Z"},
        "end":{"dateTime":"2026-09-28T17:00:00Z"},"colorId":"3","transparency":"opaque",
        "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10}]}})
}
async fn setup(items: Vec<Value>) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(
        &json!({"items":items}),
    ));
    let config = ServerConfig::from_env();
    assert!(config.google_credentials_path().is_none());
    assert!(config.google_token_cache_path().is_none());
    let state = AppState::in_memory(config)
        .await
        .unwrap()
        .with_clock(FixedClock(
            UbuTimestamp::parse("2026-09-28T08:00:00Z").unwrap(),
        ))
        .with_calendar_api(recorder.clone());
    (state, recorder)
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn reconcile(state: &AppState) -> Value {
    serde_json::to_value(
        calendar_reconciliation_service::reconcile(state, CalendarExportMode::Mock)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn capture(state: &AppState) -> Value {
    serde_json::to_value(
        calendar_capture::capture(state, CalendarExportMode::Mock)
            .await
            .unwrap(),
    )
    .unwrap()
}
fn assert_foreign(response: &Value) {
    let conflicts = response["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["external_id"], ID);
    assert_eq!(conflicts[0]["conflict_type"], "foreign");
    assert_eq!(response["status"], "observed");
    assert_eq!(
        response["diagnostics"][0]["code"],
        "capture_event_not_ownable"
    );
    assert_eq!(
        conflicts[0]["message"],
        response["diagnostics"][0]["message"]
    );
}

#[test]
fn recurring_instance_parses_without_losing_window_colour_or_summary() {
    let event = parse_event(&fixture(ID)).unwrap();
    assert_eq!(event.external_id, ID);
    assert_eq!(event.task_id, format!("task_{ID}"));
    assert_eq!(event.summary, TITLE);
    assert_eq!(event.start_at, "2026-09-28T16:30:00Z");
    assert_eq!(event.end_at, "2026-09-28T17:00:00Z");
    assert_eq!(event.color_id.as_deref(), Some("3"));
    assert!(!event.transparent);
    assert_eq!(event.reminders_minutes, vec![10]);
    println!("P1B44_TEST1={}", serde_json::to_string(&event).unwrap());
}

#[tokio::test]
async fn recurring_instance_reconciles_as_foreign_inside_the_horizon() {
    let mut outside = fixture("abc123def456ghij_20261028T163000Z");
    outside["start"]["dateTime"] = "2026-10-28T16:30:00Z".into();
    outside["end"]["dateTime"] = "2026-10-28T17:00:00Z".into();
    let (state, recorder) = setup(vec![fixture(ID), outside]).await;
    let response = reconcile(&state).await;
    assert_foreign(&response);
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    println!("P1B44_TEST2={}", response["conflicts"]);
}

// P1B-51: an event UbU cannot own is captured as occupied time, not refused.
#[tokio::test]
async fn recurring_capture_records_occupancy_under_a_minted_handle_and_only_reads_the_calendar() {
    let (state, recorder) = setup(vec![fixture(ID)]).await;
    let before = count(&state, "objects").await;
    let admissions = count(&state, "mutation_envelopes").await;
    let response = capture(&state).await;
    let after = count(&state, "objects").await;
    // Exactly one object was admitted, by exactly one envelope.
    assert_eq!(after, before + 1);
    assert_eq!(count(&state, "mutation_envelopes").await, admissions + 1);
    assert_eq!(response["captured"], 1);
    assert_eq!(response["skipped"], 0);
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"capture_occupancy_only", "message":format!("Calendar event `{ID}` cannot be owned by UbU, so its time is recorded as an occupied window that UbU will never write back to or export")} ])
    );
    assert!(!response["diagnostics"].to_string().contains(TITLE));
    assert_eq!(
        recorder.recorded_calls(),
        vec![RecordedCalendarCall::ListEvents]
    );
    // It is Static, at the event's window, with the category its colour maps to.
    let row: String =
        sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    let task: Value = serde_json::from_str(&row).unwrap();
    assert_eq!(
        task["static_window"],
        json!({"start":"2026-09-28T16:30:00Z","end":"2026-09-28T17:00:00Z"})
    );
    assert_eq!(task["category_tag"], "personal");
    assert_eq!(task["occupies_capacity"], true);
    assert_eq!(
        task["provenance"]["source"],
        json!({"source_kind":"google_calendar","source_id":ID})
    );
    // The handle is minted: nothing of the Google id is in it.
    assert!(!task["id"].as_str().unwrap().contains("abc123def456ghij"));
    println!(
        "P1B51_A_CAPTURED={}",
        json!({"objects_before":before,"objects_after":after,"task_id":task["id"],"diagnostics":response["diagnostics"]})
    );
}

#[tokio::test]
async fn occupancy_capture_never_writes_ownership_and_second_reconcile_remains_foreign() {
    let (state, recorder) = setup(vec![fixture(ID)]).await;
    let first = reconcile(&state).await;
    let before = count(&state, "projection_results").await;
    capture(&state).await;
    let after_capture = count(&state, "projection_results").await;
    assert_eq!(after_capture, before);
    assert!(
        calendar_apply::last_applied_events(state.inner().store.pool())
            .await
            .unwrap()
            .is_empty()
    );
    let second = reconcile(&state).await;
    assert_foreign(&first);
    assert_foreign(&second);
    assert_eq!(first["conflicts"], second["conflicts"]);
    // Applying the empty Plan cannot manufacture a duplicate of the foreign event.
    let preview = calendar_apply::preview(&state, false).await.unwrap();
    assert!(preview.operations.is_empty());
    calendar_apply::approve(
        &state,
        &preview.preview_id,
        ubu_core::AuthoritySource::User,
        CalendarExportMode::Mock,
    )
    .await
    .unwrap();
    assert_eq!(recorder.events(), vec![parse_event(&fixture(ID)).unwrap()]);
    assert!(recorder
        .recorded_calls()
        .iter()
        .all(|call| *call == RecordedCalendarCall::ListEvents));
    // The one object is the occupancy Task; nothing else was admitted on the way.
    assert_eq!(count(&state, "objects").await, 1);
    println!(
        "P1B44_TEST4={}",
        json!({"first":first["conflicts"],"second":second["conflicts"],"applied_records_before":before,"applied_records_after_capture":after_capture,"events_after_apply":recorder.events().len()})
    );
}

#[tokio::test]
async fn ordinary_ids_still_capture_with_unchanged_colour_diagnostics() {
    for (id, color, expected) in [
        ("aaaaa", Some("3"), None),
        ("bbbbb", Some("99"), Some(("capture_colour_unmapped", "Calendar event `bbbbb` has unmapped colour `99`; no category assigned; map that colour in Settings to assign a category"))),
        ("ccccc", None, Some(("capture_colour_absent", "Calendar event `ccccc` has no colour, so it is taken as work for UbU to schedule: a Dynamic Task of the event's length, at no fixed time"))),
    ] {
        let mut item = fixture(id);
        match color { Some(color) => item["colorId"] = color.into(), None => { item.as_object_mut().unwrap().remove("colorId"); } }
        let (state, _) = setup(vec![item]).await;
        let response = capture(&state).await;
        assert_eq!(response["captured"], 1);
        assert_eq!(response["skipped"], 0);
        assert_eq!(count(&state, "objects").await, 1);
        assert_eq!(response["diagnostics"], expected.map(|(code,message)| json!([{"code":code,"message":message}])).unwrap_or(json!([])));
        let row: String = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task'").fetch_one(state.inner().store.pool()).await.unwrap();
        let task: Value = serde_json::from_str(&row).unwrap();
        assert_eq!(task["provenance"]["source"]["source_id"], id);
        // A colour, mapped or not, is a commitment at the event's own time. No colour is Dynamic work.
        if color.is_some() {
            assert_eq!(task["static_window"], json!({"start":"2026-09-28T16:30:00Z","end":"2026-09-28T17:00:00Z"}));
            assert!(task.get("duration_estimate").is_none());
        } else {
            assert!(task.get("static_window").is_none(), "{task}");
            assert_eq!(task["duration_estimate"], json!({"type":"fixed","seconds":1800}));
        }
        assert_eq!(task["category_tag"], if color == Some("3") { json!("personal") } else { Value::Null });
    }
    let event = parse_event(&fixture("ddddd")).unwrap();
    let (tasks, diagnostics) = calendar_capture::plan_capture(
        &[event],
        &[("3".into(), None)].into_iter().collect(),
        &Default::default(),
    );
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].category_tag.is_none());
    assert_eq!(diagnostics[0].code, "capture_colour_ambiguous");
    assert_eq!(
        diagnostics[0].message,
        "Calendar event `ddddd` has a colour shared by multiple categories; no category assigned"
    );
}

#[test]
fn malformed_items_keep_skip_codes_and_identify_only_index_and_handle() {
    let mut missing = fixture("aaaaa");
    missing.as_object_mut().unwrap().remove("summary");
    let mut bad_time = fixture("bbbbb");
    bad_time["start"]["dateTime"] = "Synthetic bad timestamp content".into();
    let mut cancelled = fixture(ID);
    cancelled["status"] = "cancelled".into();
    let mut no_id = fixture("ccccc");
    no_id.as_object_mut().unwrap().remove("id");
    let (events, messages) = parse_event_list(&json!({"items":[missing,bad_time,cancelled,no_id]}));
    assert!(events.is_empty());
    assert_eq!(
        messages,
        vec![
            "list event `aaaaa` entry 0: missing or invalid summary".to_owned(),
            "list event `bbbbb` entry 1: invalid start.dateTime".to_owned(),
            format!("list event `{ID}` entry 2: cancelled event"),
            "list event `*` entry 3: missing or invalid id".to_owned()
        ]
    );
    for message in messages {
        assert!(!message.contains(TITLE));
        assert!(!message.contains("Synthetic bad timestamp content"));
        let diagnostic = list_diagnostic(message);
        assert_eq!(diagnostic.code, "calendar_event_skipped");
        println!(
            "P1B44_TEST6={}",
            serde_json::to_string(&diagnostic).unwrap()
        );
    }
}

#[tokio::test]
async fn all_day_recurring_instance_keeps_its_distinct_refusal() {
    let mut item = fixture(ID);
    item["start"] = json!({"date":"2026-09-28"});
    item["end"] = json!({"date":"2026-09-29"});
    let (state, recorder) = setup(vec![item]).await;
    let response = capture(&state).await;
    assert_eq!(response["captured"], 0);
    assert_eq!(response["skipped"], 1);
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"capture_all_day_unsupported","message":format!("list event `{ID}` entry 0: all-day event has no dateTime; it carries no duration, so it cannot be scheduled and is skipped")} ])
    );
    assert_eq!(count(&state, "objects").await, 0);
    assert_eq!(count(&state, "projection_results").await, 0);
    assert!(recorder.events().is_empty());
}
