//! Synthetic, in-process authoring and actual store-derived planning requests.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tower::ServiceExt;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, services::planning_service,
    state::AppState,
};
use ubu_store::queries;
const NOW: &str = "2026-09-28T09:00:00Z";
const VERSION: &str = "ubu.orchestrator.preference.v1";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn request(s: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = build_router(s.clone())
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
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
async fn task(s: &AppState, title: &str) -> String {
    let (status,body)=request(s,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":title,"duration_estimate":{"type":"fixed","seconds":600}})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["task_id"].as_str().unwrap().into()
}
async fn tasks(s: &AppState) -> [String; 3] {
    [
        task(s, "Synthetic A").await,
        task(s, "Synthetic B").await,
        task(s, "Synthetic C").await,
    ]
}
fn pair(a: &str, b: &str) -> Value {
    json!({"schema_version":VERSION,"task_a":a,"task_b":b,"order":"a_preferred_to_b"})
}
async fn create(s: &AppState, a: &str, b: &str) -> String {
    let (status, body) = request(s, "POST", "/preference", pair(a, b)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["schema_version"], VERSION);
    assert_eq!(body["version"], 1);
    body["preference_id"].as_str().unwrap().into()
}
async fn record(s: &AppState, id: &str) -> Value {
    let r = queries::get_current_state(s.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    json!({"version":r.version,"payload":serde_json::from_str::<Value>(&r.payload_json).unwrap()})
}
async fn values(s: &AppState) -> BTreeMap<String, f64> {
    // Assert the actual planner request, not the response's explanation records.
    let request = planning_service::build_request_from_store(s).await.unwrap();
    request.tasks.into_iter().map(|t| (t.id, t.value)).collect()
}
async fn generate(s: &AppState) -> Value {
    let (status, body) = request(s, "POST", "/planning/generate", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["plan"].is_object(), "{body}");
    body
}
async fn enable(s: &AppState, id: &str, enabled: bool, version: i64) -> (StatusCode, Value) {
    request(
        s,
        "PATCH",
        &format!("/preference/{id}"),
        json!({"schema_version":VERSION,"expected_version":version,"enabled":enabled}),
    )
    .await
}
async fn listing(s: &AppState) -> Value {
    let (status, body) = request(s, "GET", "/preferences", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    body
}
fn code(body: &Value, expected: &str) {
    assert_eq!(body["diagnostics"][0]["code"], expected, "{body}");
}
async fn count(s: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Preference'")
        .fetch_one(s.inner().store.pool())
        .await
        .unwrap()
}
async fn admissions(s: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
        .fetch_one(s.inner().store.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn captured_pair_changes_actual_request_values_with_native_provenance() {
    let s = state().await;
    let a = task(&s, "Synthetic A").await;
    let b = task(&s, "Synthetic B").await;
    let id = create(&s, &a, &b).await;
    let v = values(&s).await;
    assert!(v[&a] > v[&b]);
    assert_eq!((v[&a], v[&b]), (1.0, 0.1));
    generate(&s).await;
    let row = record(&s, &id).await;
    let p = &row["payload"];
    assert_eq!(p["acquired_method"], "user_defined");
    assert_eq!(p["acquired_date"], NOW);
    assert_eq!(p["enabled"], true);
    assert_eq!(
        p["provenance"],
        json!({"created_at":NOW,"authority_source":"user"})
    );
    assert_eq!(count(&s).await, 1);
    let list = listing(&s).await;
    assert_eq!(list["preferences"][0]["task_a_title"], "Synthetic A");
    assert_eq!(list["preferences"][0]["task_b_title"], "Synthetic B");
    let (status,_)=request(&s,"PATCH",&format!("/task/{a}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"title":"Renamed synthetic A"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listing(&s).await["preferences"][0]["task_a_title"],
        "Renamed synthetic A"
    );
    println!(
        "EVIDENCE[P1B36_test1]={}",
        json!({"task_a":a,"task_b":b,"request_values":v})
    );
}
#[tokio::test]
async fn three_task_chain_has_strictly_decreasing_request_gradient() {
    let s = state().await;
    let [a, b, c] = tasks(&s).await;
    create(&s, &a, &b).await;
    create(&s, &b, &c).await;
    let v = values(&s).await;
    assert!(v[&a] > v[&b] && v[&b] > v[&c]);
    assert_eq!((v[&a], v[&b], v[&c]), (1.0, 0.55, 0.1));
    generate(&s).await;
    println!(
        "EVIDENCE[P1B36_test2]={}",
        json!({"chain":[a,b,c],"request_values":v})
    );
}
#[tokio::test]
async fn invalid_subjects_and_untrusted_fields_admit_nothing() {
    let s = state().await;
    let [a, b, c] = tasks(&s).await;
    let (status, _) = request(
        &s,
        "POST",
        &format!("/task/{c}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let before = admissions(&s).await;
    for (body, expected) in [
        (pair(&a, &a), "preference_self_reference"),
        (
            pair(&a, "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01"),
            "preference_unknown_task",
        ),
        (pair(&a, &c), "preference_unknown_task"),
        (
            json!({"schema_version":VERSION,"objective_a":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01","objective_b":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d6e02","order":"a_preferred_to_b"}),
            "preference_objective_pair_unsupported",
        ),
    ] {
        let (status, result) = request(&s, "POST", "/preference", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        code(&result, expected);
    }
    for field in [
        "acquired_method",
        "acquired_date",
        "provenance",
        "enabled",
        "id",
    ] {
        let mut body = pair(&a, &b);
        body[field] = json!("not-authorized");
        let (status, _) = request(&s, "POST", "/preference", body).await;
        assert!(status.is_client_error());
    }
    for (version, expected) in [
        (None, "missing_schema_version"),
        (Some("wrong"), "unknown_schema_version"),
    ] {
        let mut body = pair(&a, &b);
        body.as_object_mut().unwrap().remove("schema_version");
        if let Some(v) = version {
            body["schema_version"] = v.into();
        }
        let (status, result) = request(&s, "POST", "/preference", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        code(&result, expected);
    }
    assert_eq!(count(&s).await, 0);
    assert_eq!(admissions(&s).await, before);
}
#[tokio::test]
async fn duplicate_pair_names_existing_preference_including_reversed_indifference() {
    let s = state().await;
    let [a, b, c] = tasks(&s).await;
    let id = create(&s, &a, &b).await;
    let (status, body) = request(&s, "POST", "/preference", pair(&a, &b)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "preference_duplicate_pair");
    assert!(body["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains(&id));
    let mut tie = pair(&b, &c);
    tie["order"] = "a_indifferent_to_b".into();
    let (status, body) = request(&s, "POST", "/preference", tie).await;
    assert_eq!(status, StatusCode::CREATED);
    let tie_id = body["preference_id"].as_str().unwrap();
    let mut reversed = pair(&c, &b);
    reversed["order"] = "a_indifferent_to_b".into();
    let (status, body) = request(&s, "POST", "/preference", reversed).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "preference_duplicate_pair");
    assert!(body["error"].as_str().unwrap().contains(tie_id));
    assert_eq!(count(&s).await, 2);
}
#[tokio::test]
async fn contradiction_names_existing_preference_and_preserves_it() {
    let s = state().await;
    let [a, b, _] = tasks(&s).await;
    let id = create(&s, &a, &b).await;
    let before = record(&s, &id).await;
    let mut tie = pair(&a, &b);
    tie["order"] = "a_indifferent_to_b".into();
    for request_body in [pair(&b, &a), tie] {
        let (status, body) = request(&s, "POST", "/preference", request_body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        code(&body, "preference_contradiction");
        assert!(body["error"].as_str().unwrap().contains(&id));
    }
    assert_eq!(record(&s, &id).await, before);
    assert_eq!(count(&s).await, 1);
}
#[tokio::test]
async fn closing_three_task_cycle_is_rejected_in_relationship_order() {
    let s = state().await;
    let ids = tasks(&s).await;
    let [a, b, c] = [&ids[0], &ids[2], &ids[1]]; // Deliberately not id order.
    create(&s, a, b).await;
    create(&s, b, c).await;
    let before = admissions(&s).await;
    let (status, body) = request(&s, "POST", "/preference", pair(c, a)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "preference_cycle_rejected");
    let msg = body["diagnostics"][0]["message"].as_str().unwrap();
    let sequence = msg
        .split('[')
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .split(" -> ")
        .collect::<Vec<_>>();
    assert_eq!(sequence.len(), 4);
    assert_eq!(sequence.first(), sequence.last());
    for window in sequence.windows(2) {
        assert!([
            (a.as_str(), b.as_str()),
            (b.as_str(), c.as_str()),
            (c.as_str(), a.as_str())
        ]
        .contains(&(window[0], window[1])));
    }
    assert_eq!(count(&s).await, 2);
    assert_eq!(admissions(&s).await, before);
    println!(
        "EVIDENCE[P1B36_test6]={}",
        json!({"rejection":body,"preference_count":count(&s).await})
    );
}
#[tokio::test]
async fn disabling_removes_value_effect_but_keeps_listing_and_checks_versions() {
    let s = state().await;
    let [a, b, _] = tasks(&s).await;
    let id = create(&s, &a, &b).await;
    let before = values(&s).await;
    let (status, result) = enable(&s, &id, false, 1).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["version"], 2);
    let after = values(&s).await;
    assert_eq!(before[&a], 1.0);
    assert_eq!((after[&a], after[&b]), (0.1, 0.1));
    generate(&s).await;
    let list = listing(&s).await;
    assert_eq!(list["preferences"].as_array().unwrap().len(), 1);
    assert_eq!(list["preferences"][0]["enabled"], false);
    assert_eq!(list["preferences"][0]["task_a"], a);
    assert_eq!(list["preferences"][0]["task_a_title"], "Synthetic A");
    let (status, body) = enable(&s, &id, true, 1).await;
    assert_eq!(status, StatusCode::CONFLICT);
    code(&body, "version_conflict");
    let (status, _) = request(
        &s,
        "PATCH",
        &format!("/preference/{id}"),
        json!({"schema_version":VERSION,"enabled":true}),
    )
    .await;
    assert!(status.is_client_error());
    assert_eq!(record(&s, &id).await["version"], 2);
    // The exact version permits re-enabling when no conflicting statement exists.
    assert_eq!(enable(&s, &id, true, 2).await.0, StatusCode::OK);
    assert_eq!(values(&s).await[&a], 1.0);
    println!(
        "EVIDENCE[P1B36_test7]={}",
        json!({"task_a":a,"task_b":b,"before":before,"disabled":after,"listing":list})
    );
}
#[tokio::test]
async fn disabled_edge_only_rejects_cycle_when_reenabled() {
    let s = state().await;
    let [a, b, c] = tasks(&s).await;
    let id = create(&s, &c, &a).await;
    assert_eq!(enable(&s, &id, false, 1).await.0, StatusCode::OK);
    create(&s, &a, &b).await;
    create(&s, &b, &c).await;
    let before = record(&s, &id).await;
    let writes = admissions(&s).await;
    let (status, body) = enable(&s, &id, true, 2).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "preference_cycle_rejected");
    for task in [&a, &b, &c] {
        assert!(body["error"].as_str().unwrap().contains(task));
    }
    assert_eq!(record(&s, &id).await, before);
    assert_eq!(admissions(&s).await, writes);
    assert_eq!(count(&s).await, 3);
}
#[tokio::test]
async fn deleting_withdraws_statement_and_changes_next_request() {
    let s = state().await;
    let [a, b, _] = tasks(&s).await;
    let id = create(&s, &a, &b).await;
    let before = values(&s).await;
    let (status, body) = request(&s, "DELETE", &format!("/preference/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, json!({"raw":""}));
    assert!(queries::get_current_state(s.inner().store.pool(), &id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(count(&s).await, 0);
    assert_eq!(listing(&s).await["preferences"], json!([]));
    let after = values(&s).await;
    assert_eq!((after[&a], after[&b]), (0.1, 0.1));
    assert!(before[&a] > after[&a]);
    generate(&s).await;
    assert_eq!(
        request(&s, "DELETE", &format!("/preference/{a}"), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(queries::get_current_state(s.inner().store.pool(), &a)
        .await
        .unwrap()
        .is_some());
    assert_eq!(enable(&s, &id, true, 1).await.0, StatusCode::NOT_FOUND);
    println!(
        "EVIDENCE[P1B36_test9]={}",
        json!({"task_a":a,"task_b":b,"before":before,"deleted":after})
    );
}
#[tokio::test]
async fn completing_a_subject_reports_ignored_preference_without_mutating_it() {
    let s = state().await;
    let [a, b, _] = tasks(&s).await;
    let id = create(&s, &a, &b).await;
    let before = record(&s, &id).await;
    let (status, _) = request(
        &s,
        "POST",
        &format!("/task/{a}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let plan = generate(&s).await;
    let diagnostics: Vec<_> = plan["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "preference_ignored_unknown_task")
        .collect();
    assert_eq!(diagnostics.len(), 1);
    let msg = diagnostics[0]["message"].as_str().unwrap();
    assert!(msg.contains(&id) && msg.contains(&a));
    assert_eq!(record(&s, &id).await, before);
    assert_eq!(values(&s).await[&b], 0.1);
    println!("EVIDENCE[P1B36_test10]={}", json!(diagnostics));
    assert_eq!(enable(&s, &id, false, 1).await.0, StatusCode::OK);
    let plan = generate(&s).await;
    assert!(!plan["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "preference_ignored_unknown_task"));
}
