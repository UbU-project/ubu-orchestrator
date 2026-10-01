//! P1B-53 §D: the Plan starts on a whole minute. Synthetic and offline.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    config::ServerConfig,
    planning_time::FixedClock,
    services::{calendar_client::RecordingCalendarApi, planning_service},
    state::AppState,
};

async fn setup() -> AppState {
    let state = bare().await.with_calendar_api(Arc::new(RecordingCalendarApi::new()));
    for id in [A, B, C] {
        seed(&state, id, "active", json!({"duration_estimate":{"type":"fixed","seconds":1800}})).await;
    }
    // One Static window, so that what stays put can be told from what is packed.
    seed(&state, "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f", "active", json!({"static_window":{"start":"2026-09-29T12:00:00Z","end":"2026-09-29T12:30:00Z"}})).await;
    state
}
fn at(state: &AppState, time: &str) -> AppState {
    state.clone().with_clock(FixedClock(UbuTimestamp::parse(time).unwrap()))
}
fn seconds(time: &str) -> u64 {
    UbuTimestamp::parse(time).unwrap().inner().unix_timestamp() as u64
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
/// Every Dynamic placement as (task, start, end), in Plan order.
fn dynamic(plan: &Value) -> Vec<(String, String, String)> {
    plan["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["static_anchor"] == false)
        .map(|step| (step["task_id"].as_str().unwrap().to_owned(), step["start_at"].as_str().unwrap().to_owned(), step["end_at"].as_str().unwrap().to_owned()))
        .collect()
}
fn earliest(plan: &Value) -> u64 {
    plan["plan"]["steps"].as_array().unwrap().iter().filter(|s| s["static_anchor"] == false).map(|s| s["start"].as_u64().unwrap()).min().unwrap()
}

#[tokio::test]
async fn a_plan_made_at_a_time_with_seconds_starts_on_the_following_whole_minute() {
    let state = setup().await;
    let made = at(&state, "2026-09-29T08:00:37Z");
    let window = planning_service::build_request_from_store(&made).await.unwrap().time_window.unwrap();
    // The window begins on the next minute, and its span is still exactly the configured one.
    assert_eq!(window.start, seconds("2026-09-29T08:01:00Z"));
    assert_eq!(window.end - window.start, 604_800);
    let plan = generate(&made).await;
    let placed = dynamic(&plan);
    assert_eq!(placed.len(), 3, "{plan}");
    assert_eq!(placed[0].1, "2026-09-29T08:01:00Z", "{placed:?}");
    // No Dynamic placement is earlier than the request. Only the start of the
    // packing is quantised: where the planner puts a later placement is its own
    // business, and it can fall on an odd second.
    for (_, start, _) in &placed {
        assert!(seconds(start) >= seconds("2026-09-29T08:00:37Z"), "{placed:?}");
    }
    // The Static window is where it was put, untouched.
    let anchor = plan["plan"]["steps"].as_array().unwrap().iter().find(|s| s["static_anchor"] == true).unwrap();
    assert_eq!((anchor["start_at"].as_str(), anchor["end_at"].as_str()), (Some("2026-09-29T12:00:00Z"), Some("2026-09-29T12:30:00Z")));

    // A time already on the minute is itself: it is not pushed a minute on.
    let exact = generate(&at(&state, "2026-09-29T08:05:00Z")).await;
    assert_eq!(earliest(&exact), seconds("2026-09-29T08:05:00Z"));
    // The last second of a minute rounds up to the next one, never down into the past.
    let last = generate(&at(&state, "2026-09-29T08:07:59Z")).await;
    assert_eq!(earliest(&last), seconds("2026-09-29T08:08:00Z"));
    // And the same holds at a one-day horizon.
    let one_day = AppState::in_memory(ServerConfig::from_env().with_planning_horizon_seconds(86_400))
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse("2026-09-29T08:00:37Z").unwrap()));
    let window = planning_service::build_request_from_store(&one_day).await.unwrap().time_window.unwrap();
    assert_eq!((window.start, window.end - window.start), (seconds("2026-09-29T08:01:00Z"), 86_400));
}

#[tokio::test]
async fn two_plans_within_one_minute_have_identical_dynamic_windows_and_the_preview_between_them_is_empty() {
    let state = setup().await;
    // 08:00:05: plan, preview, approve. Three events are created.
    let first_at = at(&state, "2026-09-29T08:00:05Z");
    let first = generate(&first_at).await;
    let proposed = preview(&first_at).await;
    assert_eq!(proposed["operations"].as_array().unwrap().len(), 4, "three Dynamic and one Static create: {proposed}");
    approve(&first_at, &proposed).await;

    // 08:00:50, the same minute: a second Plan. A different Plan, the same windows.
    let second_at = at(&state, "2026-09-29T08:00:50Z");
    let second = generate(&second_at).await;
    assert_ne!(first["plan"]["id"], second["plan"]["id"]);
    assert_eq!(dynamic(&first), dynamic(&second));
    assert_eq!(dynamic(&second)[0].1, "2026-09-29T08:01:00Z");
    let between = preview(&second_at).await;
    assert_eq!(between["operations"], json!([]), "a re-plan inside the same minute must write nothing: {between}");
    println!(
        "P1B53_D_SAME_MINUTE={}",
        json!({"first_plan_at":"08:00:05Z","second_plan_at":"08:00:50Z","first_dynamic":dynamic(&first),"second_dynamic":dynamic(&second),"operations_between":between["operations"]})
    );

    // 08:01:10, a minute later: the Plan legitimately differs, and the preview says so.
    let third_at = at(&state, "2026-09-29T08:01:10Z");
    let third = generate(&third_at).await;
    assert_ne!(dynamic(&third), dynamic(&second));
    assert_eq!(dynamic(&third)[0].1, "2026-09-29T08:02:00Z");
    let moved = preview(&third_at).await;
    let kinds: Vec<&str> = moved["operations"].as_array().unwrap().iter().map(|op| op["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["update", "update", "update"], "a genuine re-plan still moves genuine windows: {moved}");
    println!("P1B53_D_A_MINUTE_LATER={}", json!({"third_plan_at":"08:01:10Z","third_dynamic":dynamic(&third),"operations":kinds}));
}
