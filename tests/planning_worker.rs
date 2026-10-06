//! In-process HTTP and synthetic stores only; no interpreter or signal handler.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, services::planning_service,
    state::AppState,
};
const NOW: &str = "2026-10-07T00:00:00Z";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
async fn generate(state: &AppState) -> Value {
    let (status,body)=request(state,"POST","/planning/generate",json!({"schema_version":"planning-kernel-contract/0.1","request":{"schema_version":"planning-kernel-contract/0.1","request_id":"fixture-worker-reference","rng_seed":17,"compute_budget":{"n_rollouts":0,"top_k":3},"time_window":{"start":1,"end":1000},"tasks":[{"id":"fixture-worker-task","duration":30}]}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    body
}
fn assert_cpu(body: &Value) {
    assert_eq!(body["engine_provenance"]["backend_kind"], "cpu_reference");
    assert_eq!(
        body["engine_provenance"]["invocation_kind"],
        "in_process_cpu"
    );
    assert_eq!(
        body["engine_provenance"]["cpu_certification_status"],
        "certified"
    );
    assert_eq!(
        body["engine_provenance"]["tolerance_profile"],
        "boundary-v1"
    );
    assert!(body["engine_provenance"].get("framework").is_none());
}
#[tokio::test]
async fn absent_policy_defaults_to_cpu_and_replay_envelope_is_independent_of_scoring() {
    let state = state().await;
    let body = generate(&state).await;
    assert_cpu(&body);
    assert_eq!(body["rng_seed_echo"], 17);
    assert_eq!(body["generated_at"], NOW);
    assert_eq!(body["effective_time"], "1970-01-01T00:00:01Z");
    assert!(!body["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "planning_gpu_unavailable"));
    assert_eq!(body["plan"]["steps"][0]["start"], 1);
}
#[tokio::test]
async fn enabled_policy_reports_missing_device_stage_and_keeps_cpu_answer() {
    let state = state().await;
    let before = generate(&state).await;
    let (status, _) = request(
        &state,
        "PUT",
        "/setting/planning.gpu_enabled",
        json!({"schema_version":"ubu.orchestrator.setting.v1","value":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = generate(&state).await;
    assert_cpu(&after);
    let diagnostic = after["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "planning_gpu_unavailable")
        .unwrap();
    assert!(diagnostic["message"]
        .as_str()
        .unwrap()
        .contains("GPU compute stage not implemented"));
    assert!(diagnostic["message"]
        .as_str()
        .unwrap()
        .contains("PyTorch/CUDA compatibility unverified"));
    assert_eq!(before["selected_candidate"], after["selected_candidate"]);
}
#[tokio::test]
async fn committed_plan_retains_exact_response_provenance_and_replay() {
    let state = state().await;
    let response = generate(&state).await;
    let payload: String = sqlx::query_scalar("SELECT payload_json FROM plans WHERE id=?")
        .bind(response["plan"]["id"].as_str().unwrap())
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    let persisted: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(persisted, response["plan"]);
    assert_eq!(
        persisted["engine_provenance"],
        response["engine_provenance"]
    );
    for field in [
        "planner_version",
        "rng_seed_echo",
        "effective_time",
        "generated_at",
    ] {
        assert_eq!(persisted["replay_metadata"][field], response[field]);
    }
    assert_cpu(&persisted);
}
#[tokio::test]
async fn legacy_plan_reads_without_fabricating_certification() {
    let state = state().await;
    let response = generate(&state).await;
    let mut legacy = response["plan"].clone();
    legacy.as_object_mut().unwrap().remove("engine_provenance");
    legacy.as_object_mut().unwrap().remove("replay_metadata");
    sqlx::query("UPDATE plans SET payload_json=? WHERE id=?")
        .bind(legacy.to_string())
        .bind(legacy["id"].as_str().unwrap())
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let read = planning_service::latest_admitted_plan(&state)
        .await
        .unwrap()
        .unwrap();
    assert!(read.engine_provenance.is_none());
    assert!(read.replay_metadata.is_none());
    let (status, _) = request(&state, "GET", "/calendar/current", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
}
#[tokio::test]
async fn policy_is_boolean_setting_and_withdrawal_restores_default_off() {
    let state = state().await;
    let (status, _) = request(
        &state,
        "PUT",
        "/setting/planning.gpu_enabled",
        json!({"schema_version":"ubu.orchestrator.setting.v1","value":"true"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request(
        &state,
        "PUT",
        "/setting/planning.gpu_enabled",
        json!({"schema_version":"ubu.orchestrator.setting.v1","value":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list) = request(&state, "GET", "/settings", Value::Null).await;
    assert!(list["settings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == "planning.gpu_enabled" && s["value"] == true));
    let (status, _) = request(
        &state,
        "DELETE",
        "/setting/planning.gpu_enabled",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let body = generate(&state).await;
    assert_cpu(&body);
    assert!(!body["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "planning_gpu_unavailable"));
}
