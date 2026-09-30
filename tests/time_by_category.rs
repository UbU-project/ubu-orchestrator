//! P1B-50 §B: time by category. Synthetic and offline; every title is invented.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::state::AppState;

const SCHEMA: &str = "ubu.orchestrator.time_by_category.v1";
const ACTION: &str = "ubu.orchestrator.task_action.v1";
const D: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e73";
const E: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e74";
const F: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e75";

async fn report(state: &AppState, query: &str) -> (StatusCode, Value) {
    request(state, "GET", &format!("/reports/time-by-category?{query}"), Value::Null).await
}
async fn ok(state: &AppState, query: &str) -> Value {
    let (status, body) = report(state, query).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
fn rows(body: &Value) -> Vec<(String, u64, u64, u64, usize)> {
    body["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["category"].as_str().unwrap().into(),
                row["seconds"].as_u64().unwrap(),
                row["static_seconds"].as_u64().unwrap(),
                row["completed_seconds"].as_u64().unwrap(),
                row["task_count"].as_u64().unwrap() as usize,
            )
        })
        .collect()
}
/// A completion Log entry written the way the app and the Calendar write them.
async fn complete_at(state: &AppState, id: &str, at: &str, observed: Option<(&str, &str)>) -> String {
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let when = UbuTimestamp::parse(at).unwrap();
    let envelope = state
        .envelope_for([(UbuId::parse(id).unwrap(), VersionRef::Version(1))].into_iter().collect(), AuthoritySource::User, when)
        .unwrap();
    let mut payload = json!({"schema_version":ACTION,"action":"complete","decision":"task_completed","task_status":"completed","transition_applied":true});
    if let Some((start, end)) = observed {
        payload["source"] = json!({"source_kind":"google_calendar","source_id":"5n0q8c9h7g4k2m1p3r6t8v0a2c"});
        payload["observed_window"] = json!({"start":start,"end":end});
    }
    ubu_store::queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::log_record::NewLogRecord {
            id: log_id.clone(),
            event_type: "decision_recorded".into(),
            object_refs: json!([id]),
            payload,
            provenance: json!({"created_at":at,"authority_source":"user"}),
            created_at: at.into(),
        },
    )
    .await
    .unwrap();
    log_id
}
async fn act(state: &AppState, id: &str, action: &str) -> Value {
    let (status, body) = request(state, "POST", &format!("/task/{id}/action"), json!({"schema_version":ACTION,"action":action})).await;
    assert_eq!(status, StatusCode::OK, "{action}: {body}");
    body
}
async fn reopen(state: &AppState, id: &str, completion: &str) -> Value {
    let (status, body) = request(state, "POST", &format!("/task/{id}/reopen"), json!({"schema_version":ACTION,"completion_log_id":completion})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
const FROM: &str = "2026-09-25T00:00:00Z";
const TO: &str = "2026-09-26T00:00:00Z";
fn range() -> String {
    format!("schema_version={SCHEMA}&from={FROM}&to={TO}")
}

#[tokio::test]
async fn static_windows_contribute_their_overlap_with_the_range_whether_or_not_completed() {
    let state = bare().await;
    // Straddles the start: 22:00 to 02:00, of which two hours are inside.
    seed(&state, A, "active", json!({"category_tag":"work","tags":["work"],"static_window":{"start":"2026-09-24T22:00:00Z","end":"2026-09-25T02:00:00Z"}})).await;
    // Entirely inside, and completed: still its whole window, and not its completion.
    seed(&state, B, "completed", json!({"category_tag":"work","tags":["work"],"static_window":{"start":"2026-09-25T09:00:00Z","end":"2026-09-25T09:30:00Z"}})).await;
    complete_at(&state, B, "2026-09-25T09:30:00Z", None).await;
    // Entirely outside the range.
    seed(&state, C, "active", json!({"category_tag":"work","tags":["work"],"static_window":{"start":"2026-09-27T09:00:00Z","end":"2026-09-27T10:00:00Z"}})).await;
    // Uncategorised, straddling the end: 23:30 to 00:30, half an hour inside.
    seed(&state, D, "active", json!({"static_window":{"start":"2026-09-25T23:30:00Z","end":"2026-09-26T00:30:00Z"}})).await;
    let body = ok(&state, &range()).await;
    assert_eq!(body["schema_version"], SCHEMA);
    assert_eq!((body["from"].as_str(), body["to"].as_str()), (Some(FROM), Some(TO)));
    assert_eq!(
        rows(&body),
        vec![("work".into(), 9_000, 9_000, 0, 2), ("Uncategorized".into(), 1_800, 1_800, 0, 1)]
    );
    assert_eq!(body["total_seconds"], 10_800);
    assert_eq!(body["unmeasured"], json!([]));
    println!("P1B50_B_STATIC={body}");
}

#[tokio::test]
async fn completed_dynamic_tasks_contribute_the_observed_window_or_else_the_estimate_or_else_are_named() {
    let state = bare().await;
    // An observed window of 50 minutes beats a 25 minute estimate.
    seed(&state, A, "completed", json!({"category_tag":"grocery","tags":["grocery"],"duration_estimate":{"type":"fixed","seconds":1500}})).await;
    complete_at(&state, A, "2026-09-25T10:00:00Z", Some(("2026-09-25T09:05:00Z", "2026-09-25T09:55:00Z"))).await;
    // Fixed estimate, no window.
    seed(&state, B, "completed", json!({"category_tag":"grocery","tags":["grocery"],"duration_estimate":{"type":"fixed","seconds":600}})).await;
    complete_at(&state, B, "2026-09-25T11:00:00Z", None).await;
    // A skewed estimate: the mode, never the p95.
    seed(&state, C, "completed", json!({"category_tag":"work","tags":["work"],"duration_estimate":{"type":"shifted_lognormal_p95","min_seconds":600,"mode_seconds":1200,"p95_seconds":7200}})).await;
    complete_at(&state, C, "2026-09-25T12:00:00Z", None).await;
    // Neither: reported by name, contributing nothing.
    seed(&state, D, "completed", json!({"category_tag":"work","tags":["work"]})).await;
    complete_at(&state, D, "2026-09-25T13:00:00Z", None).await;
    // Completed outside the range: nothing, and not unmeasured either.
    seed(&state, E, "completed", json!({"category_tag":"work","tags":["work"],"duration_estimate":{"type":"fixed","seconds":3600}})).await;
    complete_at(&state, E, "2026-09-27T13:00:00Z", None).await;
    // Active and Dynamic: nothing, whatever its estimate.
    seed(&state, F, "active", json!({"category_tag":"grocery","tags":["grocery"],"duration_estimate":{"type":"fixed","seconds":3600}})).await;
    let body = ok(&state, &range()).await;
    assert_eq!(
        rows(&body),
        vec![("grocery".into(), 3_600, 0, 3_600, 2), ("work".into(), 1_200, 0, 1_200, 1)]
    );
    assert_eq!(
        body["unmeasured"],
        json!([{"task_id":D,"title":"Synthetic lunar teapot 3","reason":"completed with no observed window and no duration estimate; the time it took is not recorded"}])
    );
    assert_eq!(body["total_seconds"], 4_800);
    println!("P1B50_B_DYNAMIC={body}");
}

#[tokio::test]
async fn a_task_completed_reopened_and_completed_again_contributes_exactly_once() {
    // Through the real routes, at the fixed clock, so every log is inside the default range.
    let state = bare().await;
    seed(&state, A, "active", json!({"category_tag":"work","tags":["work"],"duration_estimate":{"type":"fixed","seconds":1500}})).await;
    let first = act(&state, A, "complete").await;
    assert_eq!(rows(&ok(&state, &format!("schema_version={SCHEMA}")).await), vec![("work".into(), 1_500, 0, 1_500, 1)]);
    reopen(&state, A, first["log_id"].as_str().unwrap()).await;
    // Reopened and left active: nothing, although its completion log is still there.
    let reopened = ok(&state, &format!("schema_version={SCHEMA}")).await;
    assert_eq!(reopened["categories"], json!([]));
    assert_eq!(reopened["total_seconds"], 0);
    println!("P1B50_B_REOPENED={}", reopened["categories"]);
    let second = act(&state, A, "complete").await;
    assert_ne!(first["log_id"], second["log_id"]);
    let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type='decision_recorded' AND json_extract(payload_json,'$.decision')='task_completed'")
        .fetch_one(state.inner().store.pool()).await.unwrap();
    assert_eq!(logs, 2, "two completion logs exist");
    let again = ok(&state, &format!("schema_version={SCHEMA}")).await;
    // Two completions in the logs; 1500 seconds, not 3000, in the report.
    assert_eq!(rows(&again), vec![("work".into(), 1_500, 0, 1_500, 1)]);
    assert_eq!(again["total_seconds"], 1_500);
    println!("P1B50_B_COMPLETED_TWICE={}", again["categories"]);
}

#[tokio::test]
async fn the_range_defaults_to_the_last_seven_days_and_a_backwards_range_is_refused() {
    let state = bare().await;
    let body = ok(&state, &format!("schema_version={SCHEMA}")).await;
    assert_eq!((body["from"].as_str(), body["to"].as_str()), (Some("2026-09-22T08:00:00Z"), Some(NOW)));
    assert_eq!(body["generated_at"], NOW);
    assert_eq!(body["categories"], json!([]));
    assert_eq!(body["total_seconds"], 0);
    // `to` alone: seven days ending there.
    let ending = ok(&state, &format!("schema_version={SCHEMA}&to=2026-09-10T00:00:00Z")).await;
    assert_eq!((ending["from"].as_str(), ending["to"].as_str()), (Some("2026-09-03T00:00:00Z"), Some("2026-09-10T00:00:00Z")));
    // The bounds are inclusive: a completion exactly at `to` counts.
    seed(&state, A, "completed", json!({"category_tag":"work","tags":["work"],"duration_estimate":{"type":"fixed","seconds":60}})).await;
    complete_at(&state, A, TO, None).await;
    assert_eq!(ok(&state, &range()).await["total_seconds"], 60);
    let (status, refused) = report(&state, &format!("schema_version={SCHEMA}&from={TO}&to={FROM}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(refused["diagnostics"][0]["code"], "time_by_category_invalid_range");
    println!("P1B50_B_BACKWARDS={}", refused["diagnostics"]);
    for (query, code) in [
        (format!("schema_version={SCHEMA}&from=yesterday"), "time_by_category_invalid_bound"),
        ("from=2026-09-25T00:00:00Z".into(), "missing_schema_version"),
        ("schema_version=ubu.orchestrator.time_by_category.v0".into(), "unknown_schema_version"),
    ] {
        let (status, body) = report(&state, &query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(body["diagnostics"][0]["code"], code, "{query}");
    }
}

#[tokio::test]
async fn rows_are_ordered_by_seconds_descending_then_category_ascending() {
    let state = bare().await;
    for (id, category, minutes) in [(A, "zebra", 30), (B, "apple", 30), (C, "mango", 45), (D, "banana", 30)] {
        let start = format!("2026-09-25T{:02}:00:00Z", 8 + (id.as_bytes()[id.len() - 1] - b'0'));
        let end = format!("2026-09-25T{:02}:{:02}:00Z", 8 + (id.as_bytes()[id.len() - 1] - b'0'), minutes);
        seed(&state, id, "active", json!({"category_tag":category,"tags":[category],"static_window":{"start":start,"end":end}})).await;
    }
    let body = ok(&state, &range()).await;
    assert_eq!(
        rows(&body).iter().map(|row| (row.0.as_str(), row.1)).collect::<Vec<_>>(),
        vec![("mango", 2_700), ("apple", 1_800), ("banana", 1_800), ("zebra", 1_800)]
    );
}
