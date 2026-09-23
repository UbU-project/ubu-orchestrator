use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, services::planning_service,
    state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};
const NOW: &str = "2026-09-22T09:00:00Z";
const STANDUP: &str = "2026-09-22T09:40:00Z";
fn id(n: u8) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e{n:02x}")
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
async fn state(uncertain: bool, second: bool) -> AppState {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()));
    admit(&state,1,"Prepare",json!({"duration_estimate":if uncertain {json!({"type":"shifted_lognormal_p95","min_seconds":600,"mode_seconds":1200,"p95_seconds":3600})} else {json!({"type":"fixed","seconds":600})}})).await;
    admit(
        &state,
        2,
        "Standup",
        json!({"static_window":{"start":STANDUP,"end":"2026-09-22T09:45:00Z"}}),
    )
    .await;
    if second {
        admit(
            &state,
            3,
            "Afternoon review",
            json!({"static_window":{"start":"2026-09-22T13:00:00Z","end":"2026-09-22T13:05:00Z"}}),
        )
        .await;
    }
    state
}
async fn post(state: &AppState, body: Value) -> Value {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/planning/generate")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let response: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(response["plan"].is_object(), "{response}");
    response
}
async fn generate(state: &AppState) -> Value {
    post(
        state,
        json!({"horizon":{"start":NOW,"end":"2026-09-22T14:00:00Z"}}),
    )
    .await
}
fn coverage(response: &Value) -> &Value {
    &response["selected_candidate"]["coverage"]
}
fn low(response: &Value) -> Vec<&Value> {
    response["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["category"] == "low_coverage")
        .collect()
}
#[tokio::test]
async fn comfortable_day_reports_full_coverage() {
    let state = state(false, false).await;
    let response = generate(&state).await;
    let coverage = coverage(&response);
    assert_eq!(coverage["scope"], "reactive_horizon");
    assert_eq!(coverage["estimate"], 1.0);
    assert_eq!(coverage["threshold_used"], 0.99);
    assert_eq!(coverage["quantization_rule"], "lateness_seconds_ceil_60");
    assert_eq!(coverage["uncovered_outcome_count"], 0);
    assert!(low(&response).is_empty());
    assert_eq!(
        response["plan"]["selected_candidate"]["coverage"],
        *coverage
    );
    let built = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(built.horizon_policy.reactive_horizon_seconds, 3600);
    assert_eq!(built.horizon_policy.branch_coverage_target, 0.99);
    let mut body = serde_json::to_value(built).unwrap();
    body["horizon_policy"] = json!({"reactive_horizon_seconds":7200,"branch_coverage_target":0.9});
    body["compute_budget"]["n_rollouts"] = json!(0);
    let typed = serde_json::from_value(body.clone()).unwrap();
    let repair = planning_service::repair_kernel_request(&typed);
    assert_eq!(repair.horizon_policy.reactive_horizon_seconds, 7200);
    assert_eq!(repair.horizon_policy.branch_coverage_target, 0.9);
    let no_rollouts = post(&state, json!({"request":body})).await;
    assert!(no_rollouts["selected_candidate"]["coverage"].is_null());
    assert!(low(&no_rollouts).is_empty());
}
#[tokio::test]
async fn uncertainty_names_commitment_without_blocking_or_recalculation() {
    let state = state(true, false).await;
    let response = generate(&state).await;
    let coverage = coverage(&response);
    let estimate = coverage["estimate"].as_f64().unwrap();
    assert!(estimate > 0.0 && estimate < 1.0);
    assert_eq!(coverage["below_threshold"], true);
    assert!(
        coverage["confidence_low"].as_f64().unwrap() <= estimate
            && coverage["confidence_high"].as_f64().unwrap() >= estimate
    );
    assert_eq!(coverage["n_rollouts"], 1000);
    let boundaries = coverage["boundaries"].as_array().unwrap();
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0]["task_id"], id(2));
    assert_eq!(boundaries[0]["summary"], "Standup");
    assert_eq!(boundaries[0]["start_at"], STANDUP);
    let findings = low(&response);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["blocking"], false);
    assert_eq!(findings[0]["subject_ref"], id(2));
    assert_eq!(findings[0]["severity"], "medium");
    let detail = findings[0]["detail"].as_str().unwrap();
    assert!(detail.ends_with("Standup") && detail.contains("60 minutes"));
    assert!(!detail.contains(&id(2)));
    assert!(response["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["blocking"] == false));
    let triggers: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE event_type='recalculation_requested'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(triggers, 0);
    println!(
        "P1B21_COVERAGE {}",
        serde_json::to_string(coverage).unwrap()
    );
    println!("P1B21_DETAIL {detail}");
    // Same HTTP path with an explicit policy and a much longer sampled tail exercises High.
    let mut supplied = serde_json::to_value(
        planning_service::build_request_from_store(&state)
            .await
            .unwrap(),
    )
    .unwrap();
    supplied["rng_seed"] = json!(42);
    supplied["tasks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["id"] == id(1))
        .unwrap()["duration_estimate"]["p95_seconds"] = json!(600000);
    // Required slow work cannot be dropped when a draw exceeds the overall horizon.
    supplied["tasks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["id"] == id(1))
        .unwrap()["mandatory"] = json!(true);
    let severe = post(&state, json!({"request":supplied})).await;
    assert_eq!(low(&severe)[0]["severity"], "high");
    assert_eq!(low(&severe)[0]["blocking"], false);
}
#[tokio::test]
async fn commitment_four_hours_out_is_outside_default_scope() {
    let state = state(true, true).await;
    let response = generate(&state).await;
    assert_eq!(
        coverage(&response)["boundaries"].as_array().unwrap().len(),
        1
    );
    assert!(response["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["task_id"] == id(3)));
    let mut supplied = serde_json::to_value(
        planning_service::build_request_from_store(&state)
            .await
            .unwrap(),
    )
    .unwrap();
    supplied["horizon_policy"] =
        json!({"reactive_horizon_seconds":18000,"branch_coverage_target":0.99});
    let wide = post(&state, json!({"request":supplied})).await;
    let boundaries = coverage(&wide)["boundaries"].as_array().unwrap();
    assert_eq!(boundaries.len(), 2);
    assert_eq!(
        boundaries[0]["uncovered_mass"],
        boundaries[1]["uncovered_mass"]
    );
    let findings = low(&wide);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["subject_ref"], id(3));
    assert!(findings[0]["detail"]
        .as_str()
        .unwrap()
        .ends_with("Afternoon review"));
    assert!(findings[0]["detail"].as_str().unwrap().contains("5 hours"));
}
