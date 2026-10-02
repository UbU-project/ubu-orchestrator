//! P1B-47: a fixture for what the mock Calendar observes. Synthetic and offline.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock,
    services::calendar_client::RecordingCalendarApi, state::AppState,
};
use ubu_store::{models::calendar_record::NewCalendarRecord, queries};

const NOW: &str = "2026-09-25T08:00:00Z";
const END: &str = "2026-09-25T20:00:00Z";
const FOREIGN: &str = "5n0q8c9h7g4k2m1p3r6t8v0a2c";

async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
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
async fn ok(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
    let (status, body) = request(state, method, uri, body).await;
    assert!(status.is_success(), "{uri}: {status} {body}");
    body
}
/// A fixture file under the system temp directory, removed when dropped.
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str, contents: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubu-p1b47-{name}-{}-{}.json",
            std::process::id(),
            UbuId::new(ObjectType::Snapshot)
        ));
        std::fs::write(&path, contents).unwrap();
        Self(path)
    }
    fn events(name: &str, events: &Value) -> Self {
        Self::new(name, &events.to_string())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn config(seed: Option<&Fixture>) -> ServerConfig {
    let config = ServerConfig::from_env();
    match seed {
        Some(fixture) => config.with_calendar_mock_events_path(&fixture.0),
        None => config,
    }
}
async fn empty(seed: Option<&Fixture>) -> AppState {
    AppState::in_memory(config(seed))
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
/// Two Static Tasks, a Plan, and the preview applied in Mock. No client is injected,
/// so every Calendar path takes the fallback this ticket changes.
async fn applied(seed: Option<&Fixture>) -> (AppState, Value) {
    let state = empty(seed).await;
    for (title, hour) in [("Synthetic breakfast", "09"), ("Synthetic stand-up", "10")] {
        ok(&state,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":title,
            "static_window":{"start":format!("2026-09-25T{hour}:00:00Z"),"end":format!("2026-09-25T{hour}:30:00Z")},
            "category_tag":"personal","tags":["personal"],"occupies_capacity":true})).await;
    }
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: NOW.into(),
            window_end: END.into(),
            payload: json!({"windows":[{"start":NOW,"end":END}]}),
            created_at: state.planning_now().to_string(),
        },
    )
    .await
    .unwrap();
    ok(&state, "POST", "/planning/generate", json!({"horizon":{"start":NOW,"end":END}})).await;
    let preview = preview(&state).await;
    assert_eq!(preview["operations"].as_array().unwrap().len(), 2);
    let result = ok(&state,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"mock"})).await;
    assert_eq!(result["status"], "applied", "{result}");
    (state, result["applied_events"].clone())
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}
async fn reconcile(state: &AppState) -> Value {
    ok(state,"POST","/projection/calendar/reconcile",json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"})).await
}
async fn capture(state: &AppState) -> Value {
    ok(state,"POST","/projection/calendar/capture",json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"})).await
}
async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
fn foreign_event() -> Value {
    json!({"external_id":FOREIGN,"summary":"Synthetic foreign meeting","start_at":"2026-09-25T13:00:00Z","end_at":"2026-09-25T13:30:00Z","color_id":null,"transparent":false,"reminders_minutes":[]})
}
fn kinds(reconciliation: &Value) -> Vec<(String, String)> {
    reconciliation["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["conflict_type"].as_str().unwrap().into(), c["external_id"].as_str().unwrap().into()))
        .collect()
}

#[tokio::test]
async fn without_the_variable_mock_mode_observes_the_applied_record_as_before() {
    assert!(config(None).calendar_mock_events_path().is_none());
    let (state, applied_events) = applied(None).await;
    assert!(state.inner().calendar_mock_events.is_none());
    // Observed is the applied record: nothing to reconcile, nothing to capture, nothing to propose.
    let reconciliation = reconcile(&state).await;
    assert_eq!(reconciliation["status"], "matched");
    assert_eq!(reconciliation["conflicts"], json!([]));
    assert_eq!(reconciliation["diagnostics"], json!([]));
    let tasks = count(&state, "objects").await;
    let captured = capture(&state).await;
    assert_eq!(
        captured,
        json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","captured":0,"updated":0,"moved":0,"resized":0,"unchanged":2,"skipped":0,"diagnostics":[]})
    );
    assert_eq!(count(&state, "objects").await, tasks);
    let next = preview(&state).await;
    assert_eq!(next["operations"], json!([]));
    assert_eq!(next["events"], applied_events);
    println!("P1B47_TEST1 reconcile={} capture={captured}", json!({"status":reconciliation["status"],"conflicts":reconciliation["conflicts"]}));
}

#[tokio::test]
async fn with_a_seed_capture_and_reconcile_observe_it_and_preview_still_diffs_the_applied_record() {
    // Learn the applied events from an unseeded state, then seed exactly one of them, recoloured.
    let (_, applied_events) = applied(None).await;
    assert_eq!(applied_events.as_array().unwrap().len(), 2);
    // Task ids differ between states, so the seeded state's own applied record is read after apply.
    let empty_seed = Fixture::events("empty", &json!([]));
    let (state, own) = applied(Some(&empty_seed)).await;
    assert_eq!(state.inner().calendar_mock_events.as_deref(), Some(&[][..]));
    // The calendar says: nothing is there. UbU believes: two events were applied.
    let reconciliation = reconcile(&state).await;
    assert_eq!(reconciliation["status"], "drifted");
    let own_ids: Vec<_> = own.as_array().unwrap().iter().map(|e| e["external_id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(
        kinds(&reconciliation),
        own_ids.iter().map(|id| ("missing".to_owned(), id.clone())).collect::<Vec<_>>()
    );
    // Preview makes no Calendar call: it diffs the desired set against the applied record,
    // so the seed does not reach it and the applied record is untouched.
    let next = preview(&state).await;
    assert_eq!(next["operations"], json!([]));
    assert_eq!(next["events"], own);
    let captured = capture(&state).await;
    assert_eq!(captured["unchanged"], 0);
    assert_eq!(captured["captured"], 0);
    println!("P1B47_TEST2 seeded=[] applied={} reconcile={}", own_ids.len(), json!({"status":reconciliation["status"],"conflicts":kinds(&reconciliation)}));
}

#[tokio::test]
async fn a_seeded_event_unknown_to_ubu_is_foreign() {
    let seed = Fixture::events("foreign", &json!([foreign_event()]));
    let state = empty(Some(&seed)).await;
    let observed = state.inner().calendar_mock_events.clone().unwrap();
    // An omitted task_id is filled as the wire parser fills it.
    assert_eq!(observed[0].task_id, format!("task_{FOREIGN}"));
    let reconciliation = reconcile(&state).await;
    assert_eq!(reconciliation["status"], "observed");
    assert_eq!(
        reconciliation["conflicts"],
        json!([{"external_id":FOREIGN,"conflict_type":"foreign","summary":"Synthetic foreign meeting","message":"this event was not created by UbU and will not be touched"}])
    );
    assert_eq!(count(&state, "objects").await, 0);
    println!("P1B47_TEST3 {}", reconciliation["conflicts"]);
}

#[tokio::test]
async fn a_live_request_beside_a_seed_is_refused_before_any_calendar_call() {
    let seed = Fixture::events("live", &json!([foreign_event()]));
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = empty(Some(&seed)).await.with_calendar_api(recorder.clone());
    let message = format!(
        "This process was started with UBU_CALENDAR_MOCK_EVENTS set to `{}`, a mock Calendar fixture, and the request asked for export_mode `live`; unset the variable and restart to use the live Calendar, or ask for `mock`",
        seed.0.display()
    );
    let preview = preview(&state).await;
    for (uri, body) in [
        ("/projection/calendar/approve", json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":preview["preview_id"],"authority_source":"user","export_mode":"live"})),
        ("/projection/calendar/capture", json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"live"})),
        ("/projection/calendar/reconcile", json!({"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"live"})),
    ] {
        let (status, refused) = request(&state, "POST", uri, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{uri}: {refused}");
        assert_eq!(
            refused["diagnostics"],
            json!([{"code":"calendar_mock_seed_with_live_export","message":message}]),
            "{uri}"
        );
        // Both are named: the variable with its file, and the mode that was asked for.
        assert!(message.contains("UBU_CALENDAR_MOCK_EVENTS") && message.contains("`live`"));
        println!("P1B47_TEST4 {uri} -> {status} {refused}");
    }
    assert!(recorder.recorded_calls().is_empty());
    for table in ["objects", "projection_results", "projection_reconciliations"] {
        assert_eq!(count(&state, table).await, 0, "{table}");
    }
    // It is the seed that refuses: without it the same request reaches the ordinary live checks.
    let (status, unseeded) = request(&empty(None).await,"POST","/projection/calendar/capture",json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"live"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unseeded["diagnostics"][0]["code"], "calendar_live_export_unconfigured");
}

#[tokio::test]
async fn a_malformed_or_unreadable_seed_refuses_startup_naming_the_path() {
    let event = foreign_event();
    let mut backwards = event.clone();
    backwards["end_at"] = json!("2026-09-25T12:00:00Z");
    let mut untitled = event.clone();
    untitled.as_object_mut().unwrap().remove("summary");
    for (name, contents, reason) in [
        ("not-json", "{ synthetic".to_owned(), "entry `<JSON>`"),
        ("not-a-list", json!({"items":[event]}).to_string(), "expected an array of events"),
        ("missing-field", json!([untitled]).to_string(), "entry `0`: missing field `summary`"),
        ("backwards", json!([event, backwards]).to_string(), "entry `1`: range start must precede end"),
        ("duplicate", json!([event, event]).to_string(), &format!("entry `1`: duplicate external_id `{FOREIGN}`")[..]),
    ] {
        let seed = Fixture::new(name, &contents);
        let error = AppState::in_memory(config(Some(&seed))).await.err().expect(name).to_string();
        assert!(error.contains(&format!("invalid mock Calendar events `{}`", seed.0.display())), "{error}");
        assert!(error.contains("UBU_CALENDAR_MOCK_EVENTS"), "{error}");
        assert!(error.contains(reason), "{name}: {error}");
        println!("P1B47_TEST6 {name}: {}", error.replace(&seed.0.display().to_string(), "<path>"));
    }
    // A path that names no file refuses as well, on the file-backed constructor too.
    let absent = std::env::temp_dir().join(format!("ubu-p1b47-absent-{}.json", std::process::id()));
    let config = ServerConfig::from_env().with_calendar_mock_events_path(&absent);
    for error in [
        AppState::in_memory(config.clone()).await.err().unwrap().to_string(),
        AppState::new(config.with_db_path("sqlite::memory:")).await.err().unwrap().to_string(),
    ] {
        assert!(error.contains(&absent.display().to_string()), "{error}");
        assert!(error.contains("entry `<file>`"), "{error}");
    }
}

// P1B-55: the seed can hold the two things a real calendar holds that capture does not take.
#[tokio::test]
async fn a_seed_can_hold_an_event_of_no_length_and_an_all_day_entry_for_capture_to_refuse() {
    let todo = json!({"external_id":"aaaaa","summary":"Synthetic: descale the kettle","start_at":"2026-09-25T13:00:00Z","end_at":"2026-09-25T13:30:00Z","color_id":null,"transparent":false,"reminders_minutes":[]});
    let instant = json!({"external_id":"bbbbb","summary":"Synthetic instant","start_at":"2026-09-25T14:00:00Z","end_at":"2026-09-25T14:00:00Z","color_id":null,"transparent":false,"reminders_minutes":[]});
    // Google's own shape, recognised by `id`: it goes through the production wire parser.
    let all_day = json!({"id":"ccccc","summary":"Synthetic all day","start":{"date":"2026-09-25"},"end":{"date":"2026-09-26"}});
    let timed = json!({"id":"ddddd","summary":"Synthetic dentist","start":{"dateTime":"2026-09-25T15:00:00Z"},"end":{"dateTime":"2026-09-25T15:30:00Z"},"colorId":"3","reminders":{"useDefault":true}});
    let seed = Fixture::events("p1b55", &json!([todo, instant, all_day, timed]));
    let state = empty(Some(&seed)).await;
    // The all-day entry is not an event the calendar observes. The other three are.
    assert_eq!(
        state.inner().calendar_mock_events.as_ref().unwrap().iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(),
        ["aaaaa", "bbbbb", "ddddd"]
    );
    assert_eq!(
        state.inner().calendar_mock_skipped,
        ["list event `ccccc` entry 2: all-day event has no dateTime; it carries no duration, so it cannot be scheduled and is skipped"]
    );
    let captured = capture(&state).await;
    assert_eq!(
        captured,
        json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","captured":2,"updated":0,"moved":0,"resized":0,"unchanged":0,"skipped":2,"diagnostics":[
            {"code":"capture_all_day_unsupported","message":"list event `ccccc` entry 2: all-day event has no dateTime; it carries no duration, so it cannot be scheduled and is skipped"},
            {"code":"capture_colour_absent","message":"Calendar event `aaaaa` has no colour, so it is taken as work for UbU to schedule: a Dynamic Task of the event's length, at no fixed time"},
            {"code":"capture_event_invalid","message":"Calendar event has an unusable title or concrete time span; skipped"}
        ]})
    );
    assert_eq!(count(&state, "objects").await, 2);
    // The same skip is reported by every read of the calendar, as a live read would.
    assert_eq!(reconcile(&state).await["diagnostics"][0]["code"], "capture_all_day_unsupported");

    // A window that runs backwards is still a mistake in the fixture, in either shape of entry
    // the loader checks itself, and an instant that is not a timestamp is refused too.
    let mut unreadable = instant.clone();
    unreadable["start_at"] = json!("synthetic");
    unreadable["end_at"] = json!("synthetic");
    let seed = Fixture::events("p1b55-unreadable", &json!([unreadable]));
    let error = AppState::in_memory(config(Some(&seed))).await.err().unwrap().to_string();
    assert!(error.contains("entry `0`: invalid range start"), "{error}");
}
