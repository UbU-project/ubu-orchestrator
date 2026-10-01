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

// The same, on a store shaped like a real week: stochastic durations, a
// Preference, routines and captured events. Quantising the start is not enough
// there. The kernel names each candidate after the request id and seeds a
// candidate's rollouts from its name, and the orchestrator minted a new request
// id for every request: twenty Plans of this store at ONE instant came back as
// seven different sets of placements. The seed and the id the kernel is given
// are now functions of what is being planned.
async fn a_week_shaped_store(horizon: u64) -> AppState {
    let wire = |id: &str, summary: &str, start: &str, end: &str, colour: &str| json!({"id":id,"summary":summary,"start":{"dateTime":start},"end":{"dateTime":end},"colorId":colour,"transparency":"opaque","reminders":{"useDefault":false,"overrides":[]}});
    let calendar = Arc::new(RecordingCalendarApi::with_wire_events(&json!({"items":[
        wire("0inv3nt3dc0unci1_20260930T000000Z","Synthetic standing teapot council","2026-09-30T00:00:00Z","2026-09-30T01:00:00Z","9"),
        wire("0inv3nt3dk3tt1edescaling","Synthetic kettle descaling","2026-09-30T02:00:00Z","2026-09-30T02:30:00Z","3"),
        wire("0inv3nt3d1ighth0uset0ur","Synthetic lighthouse tour","2026-09-30T04:00:00Z","2026-09-30T05:00:00Z","1")
    ]})));
    let state = AppState::in_memory(ServerConfig::from_env().with_planning_horizon_seconds(horizon))
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse("2026-09-29T07:59:01Z").unwrap()))
        .with_calendar_api(calendar);
    ok(&state, "POST", "/projection/calendar/capture", json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"})).await;
    // A night that begins exactly sixty minutes after the Plan's first minute, and a daily routine.
    for (title, start, seconds) in [("Synthetic night", "09:00:00", 28_800), ("Synthetic daily kelp inventory", "22:00:00", 1_800)] {
        let (status, body) = request(&state, "POST", "/objective", json!({
            "schema_version":"ubu.orchestrator.objective.v1","mode":"evergreen","title":title,
            "recurrence":{"timezone":"UTC","rule":{"kind":"daily"}},
            "routine_instance_template":{"title":title,"duration_estimate":{"type":"fixed","seconds":seconds},"nominal_start":start,
                "placement":"static","occupies_capacity":true,"tags":[],"reminder_minutes":[]}
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let lognormal = |min: u64, mode: u64, p95: u64| json!({"type":"shifted_lognormal_p95","min_seconds":min,"mode_seconds":mode,"p95_seconds":p95});
    let fixed = |seconds: u64| json!({"type":"fixed","seconds":seconds});
    let mut ids = Vec::new();
    for (title, estimate, category) in [
        ("buttons", fixed(2_700), "work"),
        ("census", lognormal(1_200, 2_400, 5_400), "work"),
        ("oat milk", fixed(1_200), "grocery"),
        ("pantry", lognormal(600, 1_200, 3_000), "grocery"),
        ("fern", fixed(1_800), "personal"),
    ] {
        let (status, body) = request(&state, "POST", "/task", json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":format!("Synthetic {title}"),"duration_estimate":estimate,"category_tag":category,"tags":[category]})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        ids.push(body["task_id"].as_str().unwrap().to_owned());
    }
    let (status, body) = request(&state, "POST", "/preference", json!({"schema_version":"ubu.orchestrator.preference.v1","task_a":ids[2],"task_b":ids[4],"order":"a_preferred_to_b"})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    state
}

#[tokio::test]
async fn twenty_plans_across_one_minute_of_a_week_shaped_store_are_one_plan_at_both_horizons() {
    for horizon in [86_400_u64, 604_800] {
        let state = a_week_shaped_store(horizon).await;
        // Twenty requests, three seconds apart, from 07:59:01 to 07:59:58: one minute, twenty request ids.
        let mut plans = Vec::new();
        let mut request_ids = std::collections::BTreeSet::new();
        for i in 0..20 {
            let plan = generate(&at(&state, &format!("2026-09-29T07:59:{:02}Z", 1 + i * 3))).await;
            request_ids.insert(plan["request_id"].as_str().unwrap().to_owned());
            plans.push(dynamic(&plan));
        }
        assert_eq!(request_ids.len(), 20, "every request still has an id of its own");
        assert_eq!(plans[0].len(), 5, "{:?}", plans[0]);
        assert_eq!(plans[0][0].1, "2026-09-29T08:00:00Z");
        for (index, plan) in plans.iter().enumerate() {
            assert_eq!(plan, &plans[0], "Plan {index} of 20 at a {horizon}-second horizon");
        }
        // And so a re-plan in that minute writes nothing: plan, approve, plan again, preview.
        let first_at = at(&state, "2026-09-29T07:59:05Z");
        generate(&first_at).await;
        approve(&first_at, &preview(&first_at).await).await;
        let second_at = at(&state, "2026-09-29T07:59:55Z");
        generate(&second_at).await;
        assert_eq!(preview(&second_at).await["operations"], json!([]), "horizon {horizon}");
        // A minute later the Plan starts a minute later, and that is a real difference.
        let later = dynamic(&generate(&at(&state, "2026-09-29T08:00:20Z")).await);
        assert_eq!(later[0].1, "2026-09-29T08:01:00Z");
        assert_ne!(later, plans[0]);
        println!("P1B53_D_WEEK_SHAPED horizon={horizon} placements={:?}", plans[0].iter().map(|(_, start, end)| format!("{}-{}", &start[11..19], &end[11..19])).collect::<Vec<_>>());
    }
}
