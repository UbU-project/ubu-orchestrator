use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-09-22T08:00:00Z";
const SHARE: &str = "static_tasks_share_committed_time";
const COLLISION: &str = "static_task_collision";
fn id(n: u8) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d7e{n:02x}")
}
fn instant(time: &str) -> String {
    format!("2026-09-22T{time}:00Z")
}
fn seconds(time: &str) -> u64 {
    UbuTimestamp::parse(instant(time))
        .unwrap()
        .inner()
        .unix_timestamp() as u64
}
fn fixed(start: &str, end: &str) -> Value {
    json!({"static_window":{"start":instant(start),"end":instant(end)}})
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn admit(state: &AppState, n: u8, title: &str, fields: Value) {
    let id = id(n);
    let mut payload = json!({"id":id,"title":title,"status":"active","provenance":{"created_at":NOW,"authority_source":"user"}});
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            UbuTimestamp::parse(NOW).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id,
            object_type: ObjectType::Task.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "test".into(),
            payload,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap();
}
async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
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
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn generate(state: &AppState) -> Value {
    request(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":instant("18:00")}}),
    )
    .await
}
fn diagnostics<'a>(response: &'a Value, code: &str) -> Vec<&'a Value> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == code)
        .collect()
}
fn step(calendar: &Value, n: u8) -> &Value {
    calendar["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id(n))
        .unwrap()
}
fn assert_static(calendar: &Value, n: u8, start: &str, end: &str) {
    let step = step(calendar, n);
    assert_eq!(step["start"], seconds(start));
    assert_eq!(step["end"], seconds(end));
    assert_eq!(step["start_at"], instant(start));
    assert_eq!(step["end_at"], instant(end));
    assert_eq!(step["static_anchor"], true);
    assert_eq!(step["occupies_capacity"], true);
}
fn assert_group(response: &Value, count: usize, carrier: u8) {
    assert_eq!(response["status"], "ok", "{response}");
    assert!(response["plan"].is_object());
    assert!(diagnostics(response, COLLISION).is_empty());
    let diagnostics = diagnostics(response, SHARE);
    assert_eq!(diagnostics.len(), 1, "{response}");
    let wording = if count == 1 {
        "Static Task happens"
    } else {
        "Static Tasks happen"
    };
    let message = format!("{count} {wording} during `{}`; the whole span is busy and every one of them stays on the Calendar", id(carrier));
    assert_eq!(diagnostics[0]["message"], message);
    for finding in response["risk_report"]["findings"].as_array().unwrap() {
        assert!(!finding.to_string().contains(SHARE));
        assert!(!finding.to_string().contains(&message));
        assert_ne!(finding["blocking"], true, "{finding}");
    }
    println!("P1B22_MESSAGE: {message}");
}

#[tokio::test]
async fn contained_chore_keeps_its_window_and_dynamic_work_stays_outside() {
    // Also exercise proper containment sharing either edge and a contained prerequisite edge.
    for (start, end) in [("12:00", "12:05"), ("09:00", "09:05"), ("14:55", "15:00")] {
        let state = state().await;
        admit(&state, 1, "Six-hour block", fixed("09:00", "15:00")).await;
        let mut chore = fixed(start, end);
        chore["blocked_by"] = json!([id(1)]);
        admit(&state, 2, "Five-minute chore", chore).await;
        admit(
            &state,
            3,
            "Dynamic work",
            json!({"duration_estimate":{"type":"fixed","seconds":1800}}),
        )
        .await;
        let response = generate(&state).await;
        assert_group(&response, 1, 1);
        let calendar = request(&state, "GET", "/calendar/current", Value::Null).await;
        assert_eq!(calendar["steps"].as_array().unwrap().len(), 3);
        assert_static(&calendar, 1, "09:00", "15:00");
        assert_static(&calendar, 2, start, end);
        let dynamic = step(&calendar, 3);
        assert_eq!(dynamic["static_anchor"], false);
        assert_eq!(
            dynamic["end"].as_u64().unwrap() - dynamic["start"].as_u64().unwrap(),
            1800
        );
        assert!(
            dynamic["end"].as_u64().unwrap() <= seconds("09:00")
                || dynamic["start"].as_u64().unwrap() >= seconds("15:00")
        );
    }
}

#[tokio::test]
async fn equal_static_windows_remain_collisions() {
    for container in [false, true] {
        let state = state().await;
        admit(&state, 1, "First pin", fixed("12:00", "12:05")).await;
        admit(&state, 2, "Second pin", fixed("12:00", "12:05")).await;
        if container {
            admit(&state, 3, "Shared container", fixed("09:00", "15:00")).await;
        }
        let response = generate(&state).await;
        assert_eq!(response["status"], "rejected", "{response}");
        assert!(response["plan"].is_null());
        let collisions = diagnostics(&response, COLLISION);
        assert_eq!(collisions.len(), 1);
        assert!(collisions[0]["message"].as_str().unwrap().contains(&id(1)));
        assert!(collisions[0]["message"].as_str().unwrap().contains(&id(2)));
    }
}

#[tokio::test]
async fn partial_overlaps_remain_collisions_even_inside_a_container() {
    for container in [false, true] {
        let state = state().await;
        admit(&state, 1, "First pin", fixed("12:00", "12:20")).await;
        admit(&state, 2, "Second pin", fixed("12:10", "12:30")).await;
        if container {
            admit(&state, 3, "Shared container", fixed("09:00", "15:00")).await;
        }
        let response = generate(&state).await;
        assert_eq!(response["status"], "rejected", "{response}");
        assert!(response["plan"].is_null());
        let collisions = diagnostics(&response, COLLISION);
        assert_eq!(collisions.len(), 1);
        assert!(collisions[0]["message"].as_str().unwrap().contains(&id(1)));
        assert!(collisions[0]["message"].as_str().unwrap().contains(&id(2)));
    }
}

#[tokio::test]
async fn three_nested_blocks_share_one_outermost_carrier() {
    let state = state().await;
    // Reverse admission order and equal starts exercise the longer-end carrier tie-break.
    admit(&state, 3, "Inner block", fixed("12:00", "13:00")).await;
    admit(&state, 2, "Middle block", fixed("09:00", "14:00")).await;
    admit(&state, 1, "Outermost block", fixed("09:00", "15:00")).await;
    let response = generate(&state).await;
    assert_group(&response, 2, 1);
    let calendar = request(&state, "GET", "/calendar/current", Value::Null).await;
    assert_eq!(calendar["steps"].as_array().unwrap().len(), 3);
    assert_static(&calendar, 1, "09:00", "15:00");
    assert_static(&calendar, 2, "09:00", "14:00");
    assert_static(&calendar, 3, "12:00", "13:00");
}
