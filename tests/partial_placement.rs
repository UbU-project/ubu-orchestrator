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
const NOW: &str = "2026-09-22T09:00:00Z";
const HOUR: &str = "2026-09-22T10:00:00Z";
fn id(n: u8) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e{n:02x}")
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
async fn task(state: &AppState, n: u8, title: &str, start: &str, end: &str) -> String {
    let id = id(n);
    admit(state,ObjectType::Task,json!({"id":id,"title":title,"status":"active","duration_estimate":{"type":"fixed","seconds":3600},"allowed_time_range":{"earliest_start":start,"latest_finish":end},"provenance":{"created_at":NOW,"authority_source":"user"}})).await;
    id
}
async fn routine(state: &AppState, n: u8, title: &str) -> String {
    let id = format!("obj_018f3c8e9b2a7c4d8f1e2a3b4c5d6e{n:02x}");
    let payload = json!({"id":id,"title":title,"status":"active","mode":"evergreen","recurrence":{"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1},"routine_instance_template":{"title":title,"nominal_start":"09:00:00","placement":"planned","allowed_local_range":{"earliest":"09:00:00","latest":"10:00:00"},"duration_estimate":{"type":"fixed","seconds":3600},"template_version":1},"provenance":{"created_at":NOW,"authority_source":"user"}});
    let _: ubu_core::core::Objective = serde_json::from_value(payload.clone()).unwrap();
    admit(state, ObjectType::Objective, payload).await;
    id
}
async fn request(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let response = build_router(state.clone())
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
    let status = response.status();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn generate(state: &AppState, end: &str) -> Value {
    let response = request(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":end}}),
    )
    .await;
    assert!(response["plan"].is_object(), "{response}");
    assert!(
        response["risk_report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["blocking"] == false),
        "{response}"
    );
    response
}
fn steps(response: &Value) -> &[Value] {
    response["plan"]["steps"].as_array().unwrap()
}
fn unplaced(response: &Value) -> &[Value] {
    response["unplaced_tasks"].as_array().unwrap()
}
async fn competition() -> (AppState, String, String) {
    let state = state().await;
    let preferred = task(&state, 1, "Preferred work", NOW, HOUR).await;
    let other = task(&state, 2, "Other work", NOW, HOUR).await;
    admit(&state,ObjectType::Preference,json!({"id":UbuId::new(ObjectType::Preference),"task_a":preferred,"task_b":other,"order":"a_preferred_to_b","acquired_method":"user_defined","acquired_date":NOW,"enabled":true,"provenance":{"created_at":NOW,"authority_source":"user"}})).await;
    (state, preferred, other)
}
async fn occurrences(state: &AppState) -> Vec<Value> {
    let rows: Vec<String>=sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence') IS NOT NULL ORDER BY id").fetch_all(state.inner().store.pool()).await.unwrap();
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).unwrap())
        .collect()
}
#[tokio::test]
async fn optional_contest_keeps_preferred_and_reports_other() {
    let (state, preferred, other) = competition().await;
    let response = generate(&state, HOUR).await;
    assert_eq!(response["status"], "partial");
    assert_eq!(steps(&response).len(), 1);
    assert_eq!(steps(&response)[0]["task_id"], preferred);
    assert_eq!(unplaced(&response).len(), 1);
    let entry = &unplaced(&response)[0];
    assert_eq!(entry["task_id"], other);
    assert_eq!(entry["summary"], "Other work");
    assert_eq!(entry["reason"], "omitted_lower_value");
    assert!(!entry["safe_alternatives"].as_array().unwrap().is_empty());
    let findings: Vec<_> = response["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["category"] == "unplaced_work")
        .collect();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["subject_ref"], other);
    assert_eq!(findings[0]["blocking"], false);
    println!(
        "P1B20_OPTIONAL {}",
        serde_json::to_string(
            &json!({"status":response["status"],"unplaced_tasks":response["unplaced_tasks"]})
        )
        .unwrap()
    );
    let calendar = request(&state, "GET", "/calendar/current", json!({})).await;
    assert_eq!(calendar["plan_id"], response["plan"]["id"]);
    assert_eq!(calendar["stale"], false);
}
#[tokio::test]
async fn materialized_routine_beats_optional_value() {
    let state = state().await;
    let objective = routine(&state, 11, "Daily routine").await;
    let optional = task(&state, 1, "Optional work", NOW, HOUR).await;
    let response = generate(&state, HOUR).await;
    assert_eq!(response["status"], "partial");
    let occurrences = occurrences(&state).await;
    assert_eq!(occurrences.len(), 1);
    assert_eq!(
        occurrences[0]["occurrence"]["routine_objective_id"],
        objective
    );
    assert_eq!(steps(&response).len(), 1);
    assert_eq!(steps(&response)[0]["task_id"], occurrences[0]["id"]);
    assert_eq!(unplaced(&response)[0]["task_id"], optional);
    assert!(response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["code"] != "mandatory_occurrence_unplaceable"));
    let built = ubu_orchestrator::services::planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let occurrence = built
        .tasks
        .iter()
        .find(|t| t.id == occurrences[0]["id"].as_str().unwrap())
        .unwrap();
    assert!(occurrence.mandatory);
    assert_eq!(occurrence.value, 0.0);
    assert_eq!(
        built.tasks.iter().find(|t| t.id == optional).unwrap().value,
        0.1
    );
}
#[tokio::test]
async fn collective_routine_overload_is_triage_and_plan_survives() {
    let state = state().await;
    routine(&state, 11, "First routine").await;
    routine(&state, 12, "Second routine").await;
    let ordinary = task(&state, 1, "Later work", HOUR, "2026-09-22T11:00:00Z").await;
    let response = generate(&state, "2026-09-22T11:00:00Z").await;
    assert_eq!(response["status"], "ok");
    let occurrences = occurrences(&state).await;
    assert_eq!(occurrences.len(), 2);
    assert_eq!(steps(&response).len(), 2);
    assert!(steps(&response).iter().any(|s| s["task_id"] == ordinary));
    let diagnostic = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "mandatory_occurrence_unplaceable")
        .unwrap();
    let stood_down = diagnostic["message"]
        .as_str()
        .unwrap()
        .split('`')
        .nth(1)
        .unwrap();
    assert!(unplaced(&response)
        .iter()
        .all(|u| u["task_id"] != stood_down));
    assert!(!steps(&response).iter().any(|s| s["task_id"] == stood_down));
    assert_eq!(
        occurrences
            .iter()
            .filter(|o| steps(&response).iter().any(|s| s["task_id"] == o["id"]))
            .count(),
        1
    );
    assert!(response["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["category"] == "routine_triage"
            && f["subject_ref"] == stood_down
            && f["blocking"] == false));
    println!(
        "P1B20_ROUTINE {}",
        serde_json::to_string(diagnostic).unwrap()
    );
}
#[tokio::test]
async fn predispatch_range_exclusion_is_in_unified_list() {
    let state = state().await;
    task(&state, 1, "Fits", NOW, HOUR).await;
    let excluded = task(&state, 2, "Beyond horizon", HOUR, "2026-09-22T11:00:00Z").await;
    let response = generate(&state, HOUR).await;
    assert_eq!(response["status"], "partial");
    let entry = unplaced(&response)
        .iter()
        .find(|u| u["task_id"] == excluded)
        .unwrap();
    assert_eq!(entry["reason"], "outside_allowed_window");
    let diagnostic = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "task_unplaceable")
        .unwrap();
    assert_eq!(entry["explanation"], diagnostic["message"]);
    assert_eq!(entry["summary"], "Beyond horizon");
}
#[tokio::test]
async fn next_action_recommends_placed_task_with_unplaced_risk() {
    let (state, preferred, other) = competition().await;
    let response = generate(&state, HOUR).await;
    assert_eq!(response["status"], "partial");
    let next = request(
        &state,
        "GET",
        "/next-action?schema_version=ubu.orchestrator.next_action.v1",
        json!({}),
    )
    .await;
    assert_eq!(next["recommendation"]["task_id"], preferred);
    assert_ne!(next["recommendation"]["task_id"], other);
    assert!(next["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["category"] == "unplaced_work"
            && f["subject_ref"] == other
            && f["blocking"] == false));
}
