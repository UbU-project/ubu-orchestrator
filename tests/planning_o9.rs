use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_orchestrator::api::planning::PlanningModeBody;
use ubu_orchestrator::api::recalculation::RECALCULATION_SCHEMA_VERSION;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::services::planning_service;
use ubu_orchestrator::state::AppState;
use ubu_store::models::calendar_record::NewCalendarRecord;
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

#[tokio::test]
async fn store_backed_request_uses_calendar_window_and_topological_order() {
    let state = test_state().await;
    let first = admit_task(&state, "First", json!({"duration_minutes": 15})).await;
    let second = admit_task(
        &state,
        "Second",
        json!({"duration_minutes": 20, "blocked_by": [first.clone()]}),
    )
    .await;
    store_calendar_window(&state, "2026-06-10T15:00:00Z", "2026-06-10T17:00:00Z").await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");

    let window = request.time_window.expect("time window");
    assert_eq!(window.start, timestamp_minutes("2026-06-10T15:00:00Z"));
    assert_eq!(window.end, timestamp_minutes("2026-06-10T17:00:00Z"));
    assert_eq!(
        request.task_graph.expect("task graph").topological_order,
        vec![first, second]
    );
    assert_eq!(request.mode, PlanningModeBody::FreshGeneration);
    assert!(request.rng_seed.is_some());
}

#[tokio::test]
async fn generate_persists_canonical_timed_plan_and_current_calendar_serves_steps() {
    let state = test_state().await;
    admit_task(&state, "Plan me", json!({"duration_minutes": 10})).await;
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", json!({})))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    let row =
        sqlx::query("SELECT status, payload_json FROM plans ORDER BY created_at DESC LIMIT 1")
            .fetch_one(state.inner().store.pool())
            .await
            .expect("plan row");
    let status: String = row.try_get("status").expect("status");
    let payload_json: String = row.try_get("payload_json").expect("payload");
    let payload: Value = serde_json::from_str(&payload_json).expect("plan json");
    assert_eq!(status, "admitted");
    assert!(payload.get("id").and_then(Value::as_str).is_some());
    assert_eq!(payload["status"], "admitted");
    assert!(payload.get("steps").and_then(Value::as_array).is_some());
    assert!(payload["steps"][0].get("start").is_some());
    assert!(payload["steps"][0].get("end").is_some());
    assert!(payload.get("tasks").is_none());

    let calendar_response = app
        .oneshot(
            Request::builder()
                .uri("/calendar/current")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("calendar response");
    assert_eq!(calendar_response.status(), StatusCode::OK);
    let body = json_body(calendar_response).await;
    assert!(body["steps"].as_array().expect("steps").len() == 1);
}

#[tokio::test]
async fn recalculation_supersedes_prior_plan_and_preserves_user_override_placement() {
    let state = test_state().await;
    let frozen = admit_task(&state, "Frozen", json!({"duration_minutes": 30})).await;
    let remaining = admit_task(
        &state,
        "Remaining",
        json!({"duration_minutes": 30, "blocked_by": [frozen.clone()]}),
    )
    .await;
    let app = ubu_orchestrator::build_router(state.clone());
    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", json!({})))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    append_user_override_log(&state, &frozen).await;
    let prior = planning_service::latest_admitted_plan(&state)
        .await
        .expect("prior query")
        .expect("prior plan");
    let frozen_before = prior
        .steps
        .iter()
        .find(|step| step.task_id == frozen)
        .expect("frozen step")
        .clone();

    let response = app
        .oneshot(json_request(
            "/planning/recalculate",
            json!({
                "schema_version": RECALCULATION_SCHEMA_VERSION,
                "triggered_at": "2026-06-10T18:00:00Z",
                "trigger_type": "user_override",
                "objects": [{"id": frozen, "object_type": "Task"}]
            }),
        ))
        .await
        .expect("recalculate response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["prior_plan_id"], prior.id);
    assert_eq!(body["plan"]["supersedes_plan_id"], prior.id);

    let new_frozen = body["plan"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["task_id"] == frozen_before.task_id)
        .expect("preserved frozen step");
    assert_eq!(new_frozen["start"], frozen_before.start);
    assert_eq!(new_frozen["end"], frozen_before.end);

    let new_remaining = body["plan"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["task_id"] == remaining)
        .expect("remaining step");
    assert!(
        new_remaining["start"].as_u64().expect("remaining start") >= frozen_before.end,
        "remaining work must not be placed before the frozen override"
    );

    let superseded: String = sqlx::query("SELECT status FROM plans WHERE id = ?")
        .bind(&prior.id)
        .fetch_one(state.inner().store.pool())
        .await
        .expect("prior row")
        .try_get("status")
        .expect("status");
    assert_eq!(superseded, "superseded");
}

#[tokio::test]
async fn planning_generate_rejects_unknown_schema_version() {
    let state = test_state().await;
    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(json_request(
            "/planning/generate",
            json!({"schema_version": "unknown"}),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "unknown_schema_version");
}

async fn test_state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state")
}

async fn admit_task(state: &AppState, title: &str, extra: Value) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let mut payload = json!({
        "id": id,
        "title": title,
        "status": "active",
        "provenance": {
            "created_at": now,
            "authority_source": "user",
            "source": {
                "source_kind": "test",
                "source_id": title
            }
        }
    });
    let map = payload.as_object_mut().expect("object");
    for (key, value) in extra.as_object().expect("extra object") {
        map.insert(key.clone(), value.clone());
    }

    queries::admit_object(
        state.inner().store.pool(),
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("task admitted");
    id
}

async fn store_calendar_window(state: &AppState, start: &str, end: &str) {
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: start.to_owned(),
            window_end: end.to_owned(),
            payload: json!({
                "windows": [{"start": start, "end": end}]
            }),
            created_at: "2026-06-10T14:30:00Z".to_owned(),
        },
    )
    .await
    .expect("calendar stored");
}

async fn append_user_override_log(state: &AppState, task_id: &str) {
    let now = UbuTimestamp::now_utc().to_string();
    queries::append_log_entry(
        state.inner().store.pool(),
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "decision_recorded".to_owned(),
            object_refs: json!([task_id]),
            payload: json!({"action": "override"}),
            provenance: json!({
                "created_at": now,
                "authority_source": "user_override"
            }),
            created_at: now,
        },
    )
    .await
    .expect("override log");
}

fn timestamp_minutes(value: &str) -> u64 {
    UbuTimestamp::parse(value)
        .expect("timestamp")
        .inner()
        .unix_timestamp() as u64
        / 60
}

fn json_request(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}
