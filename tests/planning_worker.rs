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
async fn enabled_policy_names_unsupported_strategy_and_keeps_cpu_answer() {
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
    assert!(diagnostic["message"].as_str().unwrap().contains("unsupported_strategy"));
    assert!(after["diagnostics"].as_array().unwrap().iter().any(|d|d["code"]=="planning_gpu_fallback_unsupported_strategy"));
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

fn ready_stage_environment() -> ubu_planning_worker::LocalEnvironment {
    ubu_planning_worker::LocalEnvironment {python_found:true,gpu_stage_implemented:true,torch_importable:true,torch_version:Some("2.6.0+cpu".into()),..Default::default()}
}
struct InMemoryStage { forge_padding: bool }
impl ubu_planning_worker::stage1::StageTransport for InMemoryStage {
    fn owns_compute_lock(&self) -> bool { true } // Injected eligibility, no real compute.
    fn exchange_stage1(&mut self, input:&ubu_planning_worker::stage1::StageInput) -> std::io::Result<ubu_planning_worker::stage1::StageReply> {
        use ubu_planning_worker::stage1::StageStubTransport;
        let mut reply=StageStubTransport.exchange_stage1(input)?;
        if self.forge_padding {reply.result.task_index[0][255]=0;}
        Ok(reply)
    }
}
async fn enable_worker(state:&AppState) {
    let (status,_)=request(state,"PUT","/setting/planning.gpu_enabled",json!({"schema_version":"ubu.orchestrator.setting.v1","value":true})).await;
    assert_eq!(status,StatusCode::OK);
}
#[tokio::test]
async fn policy_off_and_chunked_policy_on_never_consult_the_worker_factory() {
    for strategy in ["greedy","chunked"] {
        let state=AppState::in_memory(ServerConfig::from_env().with_planner_strategy(strategy)).await.unwrap()
            .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
            .with_planning_worker_factory(std::sync::Arc::new(|_|panic!("policy must not reach a worker")));
        let before=generate(&state).await;
        if strategy=="chunked" {
            enable_worker(&state).await;
            let after=generate(&state).await;
            assert_eq!(after["selected_candidate"],before["selected_candidate"]);
            assert!(after["diagnostics"].as_array().unwrap().iter().any(|d|d["code"]=="planning_gpu_fallback_unsupported_strategy"));
        }
    }
}
#[tokio::test]
async fn greedy_without_an_executable_factory_names_transport_absence_and_retains_cpu() {
    let state=AppState::in_memory(ServerConfig::from_env().with_planner_strategy("greedy")).await.unwrap();
    let before=generate(&state).await;
    enable_worker(&state).await;
    let after=generate(&state).await;
    assert_eq!(after["selected_candidate"],before["selected_candidate"]);
    assert!(after["diagnostics"].as_array().unwrap().iter().any(|d|d["code"]=="planning_gpu_fallback_transport_unavailable"));
}
#[tokio::test]
async fn greedy_stub_certification_and_refusal_flow_through_generate_without_any_process() {
    use ubu_planning_worker::stage1::{Stage1Strategy,plan_stage1};
    for forge_padding in [false,true] {
        let state=AppState::in_memory(ServerConfig::from_env().with_planner_strategy("greedy")).await.unwrap()
            .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()));
        let before=generate(&state).await;
        let calls=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter=calls.clone();
        let state=state.with_planning_worker_factory(std::sync::Arc::new(move|request| {
            counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let strategy=Stage1Strategy::new(true,ready_stage_environment(),true,InMemoryStage{forge_padding});
            let response=plan_stage1(request,&strategy);
            ubu_orchestrator::adapters::planning_worker::PlanningWorkerResult {response,fallback:strategy.fallback_reason(),environment:ready_stage_environment()}
        }));
        enable_worker(&state).await;
        let after=generate(&state).await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst),1);
        assert_eq!(before["selected_candidate"],after["selected_candidate"]);
        assert_eq!(before["engine_provenance"],after["engine_provenance"]); // Stubs never claim GPU execution.
        let unavailable=after["diagnostics"].as_array().unwrap().iter().find(|d|d["code"]=="planning_gpu_unavailable");
        if forge_padding {
            assert!(unavailable.unwrap()["message"].as_str().unwrap().contains("certification_failed"));
            assert!(after["diagnostics"].as_array().unwrap().iter().any(|d|d["code"]=="planning_gpu_fallback_certification_failed"));
        } else {assert!(unavailable.is_none());}
        let payload:String=sqlx::query_scalar("SELECT payload_json FROM plans WHERE id=?").bind(after["plan"]["id"].as_str().unwrap()).fetch_one(state.inner().store.pool()).await.unwrap();
        let payload:Value=serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["engine_provenance"],after["engine_provenance"]);
    }
}
#[tokio::test]
async fn every_named_kernel_reason_remains_a_public_code_and_a_private_message() {
    use ubu_planning_worker::stage1::Stage1FallbackReason::*;
    use ubu_orchestrator::adapters::planning_worker::WorkerFallback;
    for reason in [PolicyDisabled,BudgetUnjustified,PythonUnavailable,StageUnimplemented,TorchUnavailable,InterpreterStartFailed,ModuleRootUnavailable,ModulePackageUnavailable,ProbeBudgetInvalid,ProbeTimedOut,ProbeFailed,TorchVersionMismatch,ComputeLockUnavailable,InputUnsupported,TransportFailed,ReplyMismatch,CertificationFailed] {
        let diagnostics=WorkerFallback::Kernel(reason).diagnostics();
        assert_eq!(diagnostics[0].code,"planning_gpu_unavailable");
        assert!(diagnostics[0].message.ends_with(reason.as_str()));
        assert_eq!(diagnostics[1].code,format!("planning_gpu_fallback_{}",reason.as_str()));
    }
}

#[tokio::test]
async fn held_probe_facts_and_source_reach_generate_without_any_process() {
    use ubu_orchestrator::adapters::{
        planner_adapter::{CpuPlannerAdapter, PlannerAdapter},
        planning_worker::{interpreter_source_code, PlanningWorkerResult},
    };
    use ubu_planning_worker::{stage1::Stage1FallbackReason as R, InterpreterSource};
    for source in [
        InterpreterSource::EnvironmentVariable,
        InterpreterSource::Python3Fallback,
    ] {
        for reason in [
            R::PythonUnavailable,
            R::InterpreterStartFailed,
            R::ModuleRootUnavailable,
            R::ModulePackageUnavailable,
            R::ProbeBudgetInvalid,
            R::ProbeTimedOut,
            R::ProbeFailed,
            R::TorchUnavailable,
            R::TorchVersionMismatch,
        ] {
            let state =
                AppState::in_memory(ServerConfig::from_env().with_planner_strategy("greedy"))
                    .await
                    .unwrap();
            let before = generate(&state).await;
            let state = state.with_planning_worker_factory(std::sync::Arc::new(move |request| {
                let response = CpuPlannerAdapter {
                    strategy: ubu_orchestrator::config::PlannerStrategyChoice::Greedy,
                }
                .plan(request);
                let environment = ubu_planning_worker::LocalEnvironment {
                    interpreter: "synthetic-private-interpreter".into(),
                    interpreter_source: source,
                    torch_version: Some("synthetic-private-version".into()),
                    ..ready_stage_environment()
                };
                PlanningWorkerResult {
                    response,
                    fallback: Some(reason),
                    environment,
                }
            }));
            enable_worker(&state).await;
            let after = generate(&state).await;
            assert_eq!(after["selected_candidate"], before["selected_candidate"]);
            assert_cpu(&after);
            let diagnostics = after["diagnostics"].as_array().unwrap();
            assert!(diagnostics
                .iter()
                .any(|d| d["code"] == format!("planning_gpu_fallback_{}", reason.as_str())));
            let source_diagnostic = diagnostics
                .iter()
                .find(|d| d["code"] == interpreter_source_code(source))
                .unwrap();
            assert!(source_diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("synthetic-private-interpreter"));
            assert!(source_diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("synthetic-private-version"));
            for diagnostic in diagnostics {
                let code = diagnostic["code"].as_str().unwrap();
                assert!(!code.contains("synthetic-private"));
            }
        }
    }
}
#[tokio::test]
async fn successful_worker_probe_still_records_which_interpreter_was_asked() {
    use ubu_orchestrator::adapters::planning_worker::{
        interpreter_source_code, PlanningWorkerResult,
    };
    use ubu_planning_worker::{
        stage1::{plan_stage1, Stage1Strategy},
        InterpreterSource,
    };
    let state = AppState::in_memory(ServerConfig::from_env().with_planner_strategy("greedy"))
        .await
        .unwrap()
        .with_planning_worker_factory(std::sync::Arc::new(|request| {
            let environment = ubu_planning_worker::LocalEnvironment {
                interpreter: "synthetic-private-interpreter".into(),
                interpreter_source: InterpreterSource::EnvironmentVariable,
                ..ready_stage_environment()
            };
            let strategy = Stage1Strategy::new(
                true,
                environment.clone(),
                true,
                InMemoryStage {
                    forge_padding: false,
                },
            );
            PlanningWorkerResult {
                response: plan_stage1(request, &strategy),
                fallback: strategy.fallback_reason(),
                environment,
            }
        }));
    enable_worker(&state).await;
    let after = generate(&state).await;
    let diagnostics = after["diagnostics"].as_array().unwrap();
    assert!(diagnostics
        .iter()
        .any(|d| d["code"] == interpreter_source_code(InterpreterSource::EnvironmentVariable)));
    assert!(!diagnostics
        .iter()
        .any(|d| d["code"] == "planning_gpu_unavailable"));
    assert_cpu(&after);
}
