use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::planning::{GeneratePlanningRequest, PlanningRequestBody},
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::planning_service,
    state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-06-10T09:00:00Z";
fn id(n: u8) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e{n:02x}")
}
fn seconds(s: &str) -> u64 {
    UbuTimestamp::parse(s).unwrap().inner().unix_timestamp() as u64
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn admit(state: &AppState, kind: ObjectType, payload: Value) {
    let id = payload["id"].as_str().unwrap().to_owned();
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
            object_type: kind.as_str().into(),
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
async fn task(state: &AppState, n: u8, fields: Value) -> String {
    let id = id(n);
    let mut payload = json!({"id":id,"title":format!("Task {n}"),"status":"active",
        "duration_estimate":{"type":"fixed","seconds":600},
        "provenance":{"created_at":NOW,"authority_source":"user"}});
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    admit(state, ObjectType::Task, payload).await;
    id
}
async fn preference(state: &AppState, a: &str, b: &str) {
    admit(state,ObjectType::Preference,json!({"id":UbuId::new(ObjectType::Preference),
        "task_a":a,"task_b":b,"order":"a_preferred_to_b","acquired_method":"user_defined",
        "acquired_date":NOW,"enabled":true,"provenance":{"created_at":NOW,"authority_source":"user"}})).await;
}
async fn post(state: &AppState, body: Value) -> (StatusCode, Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn generate(state: &AppState, body: Value) -> Value {
    let (status, response) = post(state, body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(response["plan"].is_object(), "{response}");
    response
}
fn horizon() -> Value {
    json!({"horizon":{"start":NOW,"end":"2026-06-10T12:00:00Z"}})
}
fn range(start: &str, end: &str) -> Value {
    json!({"allowed_time_range":{"earliest_start":start,"latest_finish":end}})
}
fn step<'a>(response: &'a Value, id: &str) -> &'a Value {
    response["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap()
}
fn diagnostic(response: &Value, id: &str, code: &str) {
    let list: Vec<_> = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains(id))
        .collect();
    assert_eq!(list.len(), 1, "{response}");
    assert_eq!(list[0]["code"], code);
}

#[tokio::test]
async fn priority_reorders_dispatch_and_placement_without_double_counting() {
    let state = state().await;
    let a = task(&state, 1, json!({})).await;
    let b = task(&state, 2, json!({})).await;
    let before = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        before.task_graph.unwrap().topological_order,
        vec![a.clone(), b.clone()]
    );
    assert!(before.tasks.iter().all(|t| t.value == 0.1));
    preference(&state, &b, &a).await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        request.task_graph.as_ref().unwrap().topological_order,
        vec![b.clone(), a.clone()]
    );
    let kernel = ubu_planning_core::PlanningRequest::from(request);
    for t in kernel.tasks() {
        assert_eq!(t.priority, 1.0);
        assert_eq!(t.value, if t.id == b { 1.0 } else { 0.1 });
    }
    let response = generate(&state, json!({})).await;
    assert!(
        step(&response, &b)["start"].as_u64().unwrap()
            < step(&response, &a)["start"].as_u64().unwrap()
    );
    let priorities = response["task_priorities"].as_array().unwrap();
    assert_eq!(priorities.len(), 2);
    assert_eq!(priorities[0]["task_id"], a);
    assert_eq!(priorities[1]["task_id"], b);
    let stored: String = sqlx::query_scalar("SELECT payload_json FROM plans WHERE id = ?")
        .bind(response["plan"]["id"].as_str().unwrap())
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert!(!stored.contains("task_priorities"));
    assert!(!stored.contains("normalized_rank"));
}

#[tokio::test]
async fn unranked_tasks_use_earliest_allowed_deadline_then_id() {
    let state = state().await;
    let a = task(&state, 1, range(NOW, "2026-06-10T12:00:00Z")).await;
    let b = task(&state, 2, range(NOW, "2026-06-10T10:00:00Z")).await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        request.task_graph.unwrap().topological_order,
        vec![b.clone(), a.clone()]
    );
    assert!(request.tasks.iter().all(|t| t.value == 0.1));
    let response = generate(&state, json!({})).await;
    assert!(
        step(&response, &b)["start"].as_u64().unwrap()
            < step(&response, &a)["start"].as_u64().unwrap()
    );
}

#[tokio::test]
async fn cycle_plans_with_response_only_priorities_and_one_diagnostic() {
    let state = state().await;
    let a = task(&state, 1, json!({})).await;
    let b = task(&state, 2, json!({})).await;
    let c = task(&state, 3, json!({})).await;
    preference(&state, &a, &b).await;
    preference(&state, &b, &c).await;
    preference(&state, &c, &a).await;
    let response = generate(&state, json!({})).await;
    assert_eq!(
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] == "preference_cycle")
            .count(),
        1
    );
    let message = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "preference_cycle")
        .unwrap()["message"]
        .as_str()
        .unwrap();
    assert!(message.contains(&format!("{a}, {b}, {c}")));
    assert!(response["task_priorities"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["value"] == 1.0 && p["bucket"] == 0 && p["bucket_count"] == 1));
    println!("P1B16_CYCLE_BEGIN\n{}\nP1B16_CYCLE_END",serde_json::to_string_pretty(&json!({"task_priorities":response["task_priorities"],"diagnostics":response["diagnostics"]})).unwrap());
}

#[tokio::test]
async fn caller_value_defaults_and_invalid_values_use_named_400() {
    let state = state().await;
    let body = json!({"request_id":"caller","time_window":{"start":0,"end":3600},"tasks":[{"id":id(1),"duration":600}]});
    let request: PlanningRequestBody = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(request.tasks[0].value, 1.0);
    let response = generate(&state, json!({"request":body})).await;
    assert!(response.get("task_priorities").is_none());
    let mut invalid = body;
    invalid["tasks"][0]["value"] = json!(-1);
    // Value validation precedes correlation validation.
    invalid["tasks"][0]["correlation_groups"] = json!([{"group":"g","strength":2.0}]);
    let (status, response) = post(&state, json!({"request":invalid})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["diagnostics"][0]["code"], "invalid_task_value");
    for value in [f64::NAN, f64::INFINITY] {
        let mut request = request.clone();
        request.tasks[0].value = value;
        let error = planning_service::generate(
            state.clone(),
            GeneratePlanningRequest {
                schema_version: None,
                request: Some(request),
                horizon: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error,ubu_orchestrator::errors::AppError::Diagnostic{status:StatusCode::BAD_REQUEST,ref code,..} if code=="invalid_task_value")
        );
    }
}

#[tokio::test]
async fn allowed_ranges_intersect_horizon_and_never_widen_it() {
    let state = state().await;
    let inside = task(
        &state,
        1,
        range("2026-06-10T10:00:00Z", "2026-06-10T11:00:00Z"),
    )
    .await;
    let partial = task(
        &state,
        2,
        range("2026-06-10T08:00:00Z", "2026-06-10T09:30:00Z"),
    )
    .await;
    let response = generate(&state, horizon()).await;
    for (id, start, end) in [
        (&inside, "2026-06-10T10:00:00Z", "2026-06-10T11:00:00Z"),
        (&partial, NOW, "2026-06-10T09:30:00Z"),
    ] {
        assert!(step(&response, id)["start"].as_u64().unwrap() >= seconds(start));
        assert!(step(&response, id)["end"].as_u64().unwrap() <= seconds(end));
    }
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let window = request
        .tasks
        .iter()
        .find(|t| t.id == partial)
        .unwrap()
        .window
        .as_ref()
        .unwrap();
    assert_eq!(
        (window.start, window.end),
        (seconds(NOW), seconds("2026-06-10T09:30:00Z"))
    );
}

#[tokio::test]
async fn outside_ranges_and_oversized_tasks_are_excluded_while_others_plan() {
    let state = state().await;
    let before = task(
        &state,
        1,
        range("2026-06-10T07:00:00Z", "2026-06-10T08:00:00Z"),
    )
    .await;
    let after = task(
        &state,
        2,
        range("2026-06-10T13:00:00Z", "2026-06-10T14:00:00Z"),
    )
    .await;
    let mut short = range(NOW, "2026-06-10T09:30:00Z");
    short["duration_estimate"] = json!({"type":"fixed","seconds":3600});
    let short = task(&state, 3, short).await;
    let huge = task(
        &state,
        4,
        json!({"duration_estimate":{"type":"fixed","seconds":86401}}),
    )
    .await;
    let good = task(&state, 5, json!({})).await;
    let response = generate(&state, horizon()).await;
    for id in [before, after, short, huge] {
        diagnostic(&response, &id, "task_unplaceable");
    }
    assert_eq!(response["plan"]["steps"].as_array().unwrap().len(), 1);
    step(&response, &good);
    assert_eq!(response["task_priorities"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn lognormal_uses_kernel_placement_mode_not_p95_or_minimum() {
    let state = state().await;
    let mut fields = range(NOW, "2026-06-10T09:25:00Z");
    fields["duration_estimate"] = json!({"type":"shifted_lognormal_p95","min_seconds":600,"mode_seconds":1200,"p95_seconds":3600});
    let good = task(&state, 1, fields.clone()).await;
    fields["allowed_time_range"]["latest_finish"] = json!("2026-06-10T09:15:00Z");
    let short = task(&state, 2, fields).await;
    let response = generate(&state, horizon()).await;
    let placed = step(&response, &good);
    assert_eq!(
        placed["end"].as_u64().unwrap() - placed["start"].as_u64().unwrap(),
        1200
    );
    diagnostic(&response, &short, "task_unplaceable");
}

#[tokio::test]
async fn exclusion_reaches_dynamic_dependents_but_kept_static_breaks_chain() {
    let state = state().await;
    // Reverse ID order forces more than one pass through the exclusion fixpoint.
    let a = task(
        &state,
        3,
        json!({"duration_estimate":{"type":"fixed","seconds":86401}}),
    )
    .await;
    let b = task(&state, 2, json!({"blocked_by":[a]})).await;
    let c = task(&state, 1, json!({"blocked_by":[b]})).await;
    let s=task(&state,4,json!({"blocked_by":[a],"static_window":{"start":"2026-06-10T10:00:00Z","end":"2026-06-10T10:10:00Z"}})).await;
    let d = task(&state, 5, json!({"blocked_by":[s]})).await;
    let unrelated = task(&state, 6, json!({})).await;
    let response = generate(&state, horizon()).await;
    diagnostic(&response, &a, "task_unplaceable");
    for id in [&b, &c] {
        diagnostic(&response, id, "prerequisite_unplaceable");
    }
    for id in [&s, &d, &unrelated] {
        step(&response, id);
    }
    assert_eq!(response["plan"]["steps"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn corrupt_admitted_preference_is_an_internal_error() {
    let state = state().await;
    let a = task(&state, 1, json!({})).await;
    let b = task(&state, 2, json!({})).await;
    preference(&state, &a, &b).await;
    sqlx::query("UPDATE objects SET payload_json = '{}' WHERE object_type = 'Preference'")
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let (status, _) = post(&state, json!({})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn chunked_rescues_priority_taking_the_narrow_slot_and_greedy_omits_it() {
    for strategy in [None, Some("greedy")] {
        let mut config = ServerConfig::from_env();
        if let Some(raw) = strategy {
            config = config.with_planner_strategy(raw);
        }
        let state = AppState::in_memory(config)
            .await
            .unwrap()
            .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()));
        let a = task(&state, 1, range(NOW, "2026-06-10T09:10:00Z")).await;
        let b = task(&state, 2, json!({})).await;
        preference(&state, &b, &a).await;
        let (status, response) = post(&state, horizon()).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        println!(
            "P1B17_RESCUE_{}_BEGIN\n{}\nP1B17_RESCUE_END",
            strategy.unwrap_or("chunked"),
            serde_json::to_string_pretty(
                &json!({"steps":response["plan"]["steps"],"diagnostics":response["diagnostics"]})
            )
            .unwrap()
        );
        if strategy.is_none() {
            assert_eq!(
                state.inner().planner_strategy,
                ubu_orchestrator::config::PlannerStrategyChoice::Chunked
            );
            assert_eq!(step(&response, &a)["start"], seconds(NOW));
            assert_eq!(step(&response, &a)["end"], seconds("2026-06-10T09:10:00Z"));
            assert!(
                step(&response, &b)["start"].as_u64().unwrap() >= seconds("2026-06-10T09:10:00Z")
            );
        } else {
            assert_eq!(response["status"], "partial");
            assert!(response["unplaced_tasks"].as_array().unwrap().iter().any(|u| u["task_id"] == a));
            assert!(response["plan"].is_object(), "{response}");
            assert!(response["plan"]["steps"].as_array().unwrap().iter().all(|s| s["task_id"] != a));
            assert_eq!(step(&response, &b)["task_id"], b);
        }
    }
}

#[tokio::test]
async fn recalculation_uses_configured_strategy_after_priority_changes() {
    use ubu_orchestrator::services::recalculation_service;
    for strategy in ["chunked", "greedy"] {
        let state = AppState::in_memory(ServerConfig::from_env().with_planner_strategy(strategy))
            .await
            .unwrap()
            .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()));
        let a = task(&state, 1, range(NOW, "2026-06-10T09:10:00Z")).await;
        let b = task(&state, 2, json!({})).await;
        // Without a preference, both strategies can admit the prior Plan.
        let prior = generate(&state, horizon()).await;
        preference(&state, &b, &a).await;
        let response = recalculation_service::recalculate_from_request(
            state.clone(),
            serde_json::from_value(json!({
                "triggered_at":NOW,"trigger_type":"worker_request","objects":[]
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let response = serde_json::to_value(response).unwrap();
        assert_eq!(response["prior_plan_id"], prior["plan"]["id"]);
        if strategy == "chunked" {
            assert_eq!(step(&response, &a)["start"], seconds(NOW));
            assert_eq!(step(&response, &a)["end"], seconds("2026-06-10T09:10:00Z"));
            assert!(
                step(&response, &b)["start"].as_u64().unwrap() >= seconds("2026-06-10T09:10:00Z")
            );
        } else {
            assert!(response["plan"].is_object(), "{response}");
            assert!(response["plan"]["steps"].as_array().unwrap().iter().all(|s| s["task_id"] != a));
            assert_eq!(step(&response, &b)["task_id"], b);
        }
    }
}
