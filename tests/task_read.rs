//! Synthetic, in-process Task reads. Nothing reaches the network.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    api::planning::TimeWindowBody, build_router, config::ServerConfig, planning_time::FixedClock,
    services::routine_service::materialize, state::AppState,
};

const NOW: &str = "2026-09-28T09:00:00Z";
const CAPTURE: &str = "ubu.orchestrator.task_capture.v1";
const READ: &str = "ubu.orchestrator.task_read.v1";
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
async fn task(s: &AppState, mut fields: Value) -> String {
    fields["schema_version"] = json!(CAPTURE);
    let (status, body) = request(s, "POST", "/task", fields).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["task_id"].as_str().unwrap().into()
}
async fn list(s: &AppState, query: &str) -> Value {
    let (status, body) = request(s, "GET", &format!("/tasks{query}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["schema_version"], READ);
    body
}
fn ids(list: &Value) -> Vec<String> {
    list["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task_id"].as_str().unwrap().to_owned())
        .collect()
}
fn code(body: &Value, expected: &str) {
    assert_eq!(body["diagnostics"][0]["code"], expected, "{body}");
}

#[tokio::test]
async fn the_list_defaults_to_active_filters_by_status_and_is_stable() {
    let s = state().await;
    let a = task(
        &s,
        json!({"title":"Synthetic A","duration_estimate":{"type":"fixed","seconds":600},
        "category_tag":"home","tags":["home"],"due_at":"2026-10-01T17:00:00Z"}),
    )
    .await;
    let b = task(&s, json!({"title":"Synthetic B"})).await;
    let c = task(&s, json!({"title":"Synthetic C"})).await;
    let (status, body) = request(
        &s,
        "POST",
        &format!("/task/{b}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let active = list(&s, "").await;
    assert_eq!(active["status"], "active");
    let mut expected = vec![a.clone(), c.clone()];
    // One creation instant under the fixed clock, so id decides the order.
    expected.sort();
    assert_eq!(ids(&active), expected);
    assert_eq!(active, list(&s, "").await);
    assert_eq!(active, list(&s, "?status=active").await);
    assert_eq!(
        active,
        list(&s, &format!("?status=active&schema_version={READ}")).await
    );
    let first = active["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["task_id"] == a)
        .unwrap();
    assert_eq!(
        *first,
        json!({"task_id":a,"title":"Synthetic A","status":"active","version":1,"placement":"planned",
            "duration_estimate":{"type":"fixed","seconds":600},"due_at":"2026-10-01T17:00:00Z",
            "category_tag":"home","is_routine_occurrence":false})
    );
    let completed = list(&s, "?status=completed").await;
    assert_eq!(ids(&completed), vec![b]);
    assert_eq!(completed["tasks"][0]["status"], "completed");
    assert_eq!(completed, list(&s, "?status=completed").await);
    assert_eq!(list(&s, "?status=moot").await["tasks"], json!([]));
    let (status, body) = request(&s, "GET", "/tasks?status=ready", Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "unknown_task_status");
    let (status, body) = request(&s, "GET", "/tasks?schema_version=other", Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    code(&body, "unknown_schema_version");
}

#[tokio::test]
async fn a_routine_occurrence_is_marked_and_still_not_editable() {
    let s = state().await;
    let (status, body) = request(&s, "POST", "/objective", json!({
        "schema_version":"ubu.orchestrator.objective.v1","title":"Synthetic stretch","mode":"evergreen",
        "recurrence":{"timezone":"America/New_York","rule":{"kind":"daily"}},
        "routine_instance_template":{"title":"Synthetic stretch","duration_estimate":{"type":"fixed","seconds":300},
            "nominal_start":"12:00:00","placement":"static"}})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let objective = body["objective_id"].as_str().unwrap().to_owned();
    let ordinary = task(&s, json!({"title":"Synthetic A"})).await;
    let start = UbuTimestamp::parse(NOW).unwrap().inner().unix_timestamp() as u64;
    let horizon = TimeWindowBody {
        start,
        end: start + 86400,
    };
    materialize(&s, &horizon, start).await.unwrap();
    let listed = list(&s, "").await;
    let tasks = listed["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2, "{listed}");
    let occurrence = tasks
        .iter()
        .find(|t| t["is_routine_occurrence"] == true)
        .unwrap();
    assert_eq!(occurrence["title"], "Synthetic stretch");
    assert_eq!(occurrence["objective_id"], objective);
    assert_eq!(occurrence["placement"], "static");
    let plain = tasks.iter().find(|t| t["task_id"] == ordinary).unwrap();
    assert_eq!(plain["is_routine_occurrence"], false);
    let id = occurrence["task_id"].as_str().unwrap();
    let (status, body) = request(&s, "GET", &format!("/task/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["is_routine_occurrence"], true);
    assert_eq!(body["version"], occurrence["version"]);
    assert_eq!(
        body["payload"]["occurrence"]["routine_objective_id"],
        objective
    );
    let (status, body) = request(
        &s,
        "PATCH",
        &format!("/task/{id}"),
        json!({"schema_version":CAPTURE,"expected_version":occurrence["version"],"title":"Vanishes"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    code(&body, "routine_occurrence_not_editable");
    assert_eq!(list(&s, "").await, listed);
}

#[tokio::test]
async fn checklist_children_report_their_container_and_ordinary_tasks_do_not() {
    let s = state().await;
    let origin = task(
        &s,
        json!({"title":"Repair gate","duration_estimate":{"type":"fixed","seconds":3600}}),
    )
    .await;
    let ordinary = task(&s, json!({"title":"Synthetic A"})).await;
    let child =
        |title: &str| json!({"title":title,"duration_estimate":{"type":"fixed","seconds":1800}});
    let (status, decomposed) = request(
        &s,
        "POST",
        &format!("/task/{origin}/decompose"),
        json!({"schema_version":"ubu.orchestrator.container.v1","expected_version":1,
            "children":[child("Buy hinges"),child("Hang the gate")],"segment_split_points":[]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decomposed}");
    let container = decomposed["container_id"].as_str().unwrap();
    let listed = list(&s, "").await;
    let tasks = listed["tasks"].as_array().unwrap();
    for child in decomposed["child_task_ids"].as_array().unwrap() {
        let row = tasks.iter().find(|t| t["task_id"] == *child).unwrap();
        assert_eq!(row["container_id"], container, "{listed}");
    }
    assert_eq!(decomposed["child_task_ids"].as_array().unwrap().len(), 2);
    let plain = tasks.iter().find(|t| t["task_id"] == ordinary).unwrap();
    assert!(plain.get("container_id").is_none(), "{listed}");
    assert!(tasks.iter().all(|t| t["task_id"] != origin), "{listed}");
    // Undo supersedes the Container, so nothing still claims membership.
    let (status, body) = request(
        &s,
        "POST",
        &format!("/container/{container}/undo"),
        json!({"schema_version":"ubu.orchestrator.container.v1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = list(&s, "").await;
    assert!(
        listed["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t.get("container_id").is_none()),
        "{listed}"
    );
}

#[tokio::test]
async fn an_unknown_task_is_not_found() {
    let s = state().await;
    let known = task(&s, json!({"title":"Synthetic A"})).await;
    let (status, body) = request(&s, "GET", &format!("/task/{known}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["schema_version"], READ);
    assert_eq!(body["version"], 1);
    assert_eq!(body["status"], "active");
    assert_eq!(body["is_routine_occurrence"], false);
    assert_eq!(body["payload"]["title"], "Synthetic A");
    for id in [
        UbuId::new(ObjectType::Task).to_string(),
        UbuId::new(ObjectType::Objective).to_string(),
        "not-an-id".to_owned(),
    ] {
        let (status, body) = request(&s, "GET", &format!("/task/{id}"), Value::Null).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        code(&body, "unknown_task");
    }
}
