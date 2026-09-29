//! Synthetic route tests. Real transport lives only in the executable, never here.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        advisory_wire::{self as wire, Failure},
        setting_authoring,
    },
    state::AppState,
};

const NOW: &str = "2026-09-29T08:00:00Z";
const A: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
const B: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e71";
const C: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e72";
const D: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e73";
#[derive(Default)]
struct StubTransport {
    submissions: Mutex<Vec<LocalAdvisorySubmission>>,
    failure: Option<Failure>,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(sub.clone());
        if let Some(reason) = self.failure {
            return Ok(match reason {
                Failure::TooLarge => wire::interpret(
                    sub,
                    200,
                    &vec![b'x'; sub.result_size_limit_bytes as usize + 1],
                ),
                _ => wire::failed(sub, reason),
            });
        }
        let proposals: Vec<_> = sub
            .payload
            .as_array()
            .unwrap()
            .iter()
            .map(|task| json!({"id":task["id"],"category_tag":"work","confidence":0.8}))
            .collect();
        let body = json!({"done":true,"response":json!({"proposals":proposals}).to_string()});
        Ok(wire::interpret(
            sub,
            200,
            &serde_json::to_vec(&body).unwrap(),
        ))
    }
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
fn inject(state: AppState, stub: Arc<StubTransport>) -> AppState {
    state.with_advisory_transport_factory(Arc::new(move |endpoint| {
        assert_eq!(endpoint, "http://127.0.0.1:11434");
        stub.clone()
    }))
}
async fn configure(state: &AppState) {
    setting_authoring::put(state, "advisory.model", json!("synthetic-model:1"))
        .await
        .unwrap();
    setting_authoring::put(state, "advisory.endpoint", json!("http://127.0.0.1:11434"))
        .await
        .unwrap();
}
async fn seed(state: &AppState, id: &str, status: &str, category: Option<&str>) {
    let id = UbuId::parse(id).unwrap();
    let now = state.planning_now();
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    let mut tags = vec!["synthetic-hidden-tag"];
    if let Some(tag) = category {
        tags.push(tag);
    }
    let mut payload = json!({"id":id,"title":format!("Synthetic lunar teapot {}",id.as_str().chars().last().unwrap()),"status":status,"tags":tags,
        "provenance":{"created_at":now,"authority_source":"user"}});
    if let Some(category) = category {
        payload["category_tag"] = category.into();
    }
    ubu_store::queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::object_record::NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::Task.as_str().into(),
            version: 1,
            status: status.into(),
            compartment_label: "synthetic-private-compartment".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
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
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn run(state: &AppState, limit: Option<usize>) -> Value {
    let (status,response)=request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"suggest_tags","limit":limit})).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response
}
async fn count(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM advisory_candidates")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn action(state: &AppState, id: &str, action: &str, version: u64) -> Value {
    let body = match action {
        "reject" => {
            json!({"observed_version":version,"reason":"Synthetic operator correction","retention_policy":"retain"})
        }
        "resurface" => json!({"observed_version":version,"trigger":"user_request"}),
        _ => json!({"observed_version":version}),
    };
    let (status, body) = request(
        state,
        "POST",
        &format!("/advisory/candidate/{id}/{action}"),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn unset_model_is_named_and_nothing_is_enqueued() {
    let state = state().await;
    setting_authoring::put(&state, "advisory.endpoint", json!("http://127.0.0.1:11434"))
        .await
        .unwrap();
    let response = run(&state, None).await;
    assert_eq!(response["status"], "unconfigured");
    assert_eq!(response["selected"], json!([]));
    assert_eq!(count(&state).await, 0);
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"advisory_unconfigured","message":"advisory.model is not configured; set it in Setup before running SuggestTags"}])
    );
    let (_, settings) = request(&state, "GET", "/settings", Value::Null).await;
    assert_eq!(
        settings["advisory"][0],
        json!({"name":"advisory.model","value":null,"origin":"unconfigured"})
    );
    assert_eq!(settings["advisory"][1]["origin"], "setting");
    println!("P1B45_TEST1={}", response["diagnostics"]);
}
#[tokio::test]
async fn unset_endpoint_is_named_and_settings_reject_non_loopback_or_empty_values() {
    let state = state().await;
    setting_authoring::put(&state, "advisory.model", json!("synthetic-model:1"))
        .await
        .unwrap();
    let response = run(&state, None).await;
    assert_eq!(count(&state).await, 0);
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"advisory_unconfigured","message":"advisory.endpoint is not configured; set it in Setup before running SuggestTags"}])
    );
    for endpoint in [
        "https://127.0.0.1:11434",
        "http://localhost:11434",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://127.0.0.1:65536",
        "http://127.0.0.1:11434/path",
        "http://127.0.0.1:11434?x=y",
        "http://127.0.0.1:11434#x",
        "http://user:pass@127.0.0.1:11434",
        "http://example.invalid:11434",
    ] {
        let (status, body) = request(
            &state,
            "PUT",
            "/setting/advisory.endpoint",
            json!({"schema_version":"ubu.orchestrator.setting.v1","value":endpoint}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["diagnostics"][0]["code"],
            "setting_invalid_advisory_endpoint"
        );
    }
    for name in ["advisory.endpoint", "advisory.model"] {
        for value in [json!(" "), json!(42), Value::Null] {
            assert!(setting_authoring::put(&state, name, value).await.is_err());
        }
    }
    assert!(
        setting_authoring::put(&state, "advisory.other", json!("synthetic"))
            .await
            .is_err()
    );
    configure(&state).await;
    setting_authoring::delete(&state, "advisory.endpoint")
        .await
        .unwrap();
    assert!(
        setting_authoring::advisory_value(&state, "advisory.endpoint")
            .await
            .unwrap()
            .is_none()
    );
    println!("P1B45_TEST2={}", response["diagnostics"]);
}
#[tokio::test]
async fn stub_run_selects_only_uncategorised_active_tasks_and_enqueues_candidates() {
    let stub = Arc::new(StubTransport::default());
    let state = inject(state().await, stub.clone());
    configure(&state).await;
    for (id, status, category) in [
        (B, "active", None),
        (D, "completed", None),
        (C, "active", Some("personal")),
        (A, "active", None),
    ] {
        seed(&state, id, status, category).await;
    }
    let response = run(&state, None).await;
    assert_eq!(response["status"], "ok");
    assert_eq!(response["candidates_enqueued"], 2);
    assert_eq!(count(&state).await, 2);
    assert_eq!(
        response["selected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [A, B]
    );
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    for candidate in queue["candidates"].as_array().unwrap() {
        assert_eq!(
            candidate["candidate"]["proposing_actor"]["model_or_tool_name"],
            "synthetic-model:1"
        );
        assert_eq!(
            candidate["candidate"]["normalized_proposal"]["operation"],
            "set_category"
        );
    }
    assert_eq!(queue["target_titles"][A], "Synthetic lunar teapot 0");
    assert_eq!(response["candidate_ids"].as_array().unwrap().len(), 2);
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn submission_and_wire_request_carry_only_task_ids_and_titles_as_task_data() {
    let stub = Arc::new(StubTransport::default());
    let state = inject(state().await, stub.clone());
    configure(&state).await;
    seed(&state, A, "active", None).await;
    run(&state, None).await;
    let captured = stub.submissions.lock().unwrap();
    let sub = &captured[0];
    assert_eq!(
        sub.payload,
        json!([{"id":A,"title":"Synthetic lunar teapot 0"}])
    );
    assert!(sub.submission_id.starts_with("advcand_"));
    assert_eq!(sub.authority.granted.len(), 2);
    assert_eq!(
        sub.authority.authority_source,
        AuthoritySource::AutomationWorker
    );
    assert!(sub.authority.deadline.is_none());
    assert_eq!(sub.expected_result_schema, wire::TAG_RESULT_SCHEMA);
    assert_eq!(sub.timeout_ms, 120_000);
    assert_eq!(sub.result_size_limit_bytes, 262_144);
    assert_eq!(sub.compute_budget.max_cpu_ms, 120_000);
    assert_eq!(sub.compute_budget.max_memory_bytes, 536_870_912);
    assert!(!sub.partial_results_allowed);
    assert!(sub.causal_parents.is_empty());
    assert!(sub.observed_policy_versions.is_empty());
    assert!(sub.input_digests.is_empty());
    assert!(sub.execution_context.is_none());
    assert_eq!(
        sub.origin_device_id,
        state.inner().device_registration.device_id
    );
    assert_eq!(sub.submitted_at.to_string(), NOW);
    assert_eq!(sub.provider_config.provider_name, "ollama");
    assert_eq!(sub.provider_config.provider_version, "api/generate");
    assert_eq!(sub.provider_config.model_name, "synthetic-model:1");
    assert_eq!(sub.provider_config.model_version, "unspecified");
    assert!(sub.provider_config.prompt_template_digest.is_none());
    let body = wire::request_body(sub).unwrap();
    // Five fields in P1B-45; P1B-46 added `think`.
    assert_eq!(body.as_object().unwrap().len(), 6);
    assert_eq!(body["stream"], false);
    assert_eq!(body["think"], false);
    assert_eq!(
        serde_json::from_str::<Value>(body["prompt"].as_str().unwrap()).unwrap(),
        sub.payload
    );
    for forbidden in [
        "provenance",
        "compartment",
        "synthetic-hidden-tag",
        "origin_device_id",
        "authority_source",
        "objective",
        "input_digests",
    ] {
        assert!(!body.to_string().contains(forbidden), "{forbidden}");
    }
    println!("P1B45_TEST4={}", sub.payload);
}
#[tokio::test]
async fn selection_limit_is_bounded_stable_and_named_exactly() {
    let state = inject(state().await, Arc::new(StubTransport::default()));
    configure(&state).await;
    seed(&state, B, "active", None).await;
    seed(&state, A, "active", None).await;
    let response = run(&state, Some(1)).await;
    assert_eq!(
        response["selected"],
        json!([{"id":A,"title":"Synthetic lunar teapot 0"}])
    );
    assert_eq!(response["candidates_enqueued"], 1);
    for limit in [0, 26] {
        let (status,_)=request(&state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"suggest_tags","limit":limit})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let second = run(&state, Some(1)).await;
    assert_eq!(second["candidates_enqueued"], 0);
    assert_eq!(count(&state).await, 1);
}
#[tokio::test]
async fn connection_and_timeout_failures_are_200_diagnostics_without_candidates() {
    for reason in [Failure::Connection, Failure::Timeout, Failure::Http] {
        let state = inject(
            state().await,
            Arc::new(StubTransport {
                failure: Some(reason),
                ..Default::default()
            }),
        );
        configure(&state).await;
        seed(&state, A, "active", None).await;
        let response = run(&state, None).await;
        assert_eq!(response["candidates_enqueued"], 0);
        assert_eq!(count(&state).await, 0);
        assert_ne!(response["status"], "ok");
        assert!(!response["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .is_empty());
        println!("P1B45_TEST6={}", response["diagnostics"]);
    }
}
#[tokio::test]
async fn oversized_and_malformed_results_fail_closed() {
    let state = inject(
        state().await,
        Arc::new(StubTransport {
            failure: Some(Failure::TooLarge),
            ..Default::default()
        }),
    );
    configure(&state).await;
    seed(&state, A, "active", None).await;
    let response = run(&state, None).await;
    assert_eq!(
        response["diagnostics"][0]["code"],
        "advisory_result_too_large"
    );
    assert_eq!(response["candidates_enqueued"], 0);
    assert_eq!(count(&state).await, 0);
    let mut bytes = vec![0; 3];
    assert!(wire::append_chunk(&mut bytes, &[0; 2], 4).is_err());
    assert_eq!(bytes.len(), 3);
    let tasks = ubu_orchestrator::services::suggest_tags::select(&state, 5)
        .await
        .unwrap();
    let sub =
        ubu_orchestrator::services::suggest_tags::submission(&state, &tasks, "synthetic-model:1")
            .await
            .unwrap();
    for proposal in [
        json!({"id":B,"category_tag":"work","confidence":0.8}),
        json!({"id":A,"category_tag":"","confidence":0.8}),
        json!({"id":A,"category_tag":"work","confidence":1.1}),
        json!({"id":A,"category_tag":"work","confidence":0.8,"provenance":"invented"}),
    ] {
        let body = json!({"done":true,"response":json!({"proposals":[proposal]}).to_string()});
        let result = wire::interpret(&sub, 200, &serde_json::to_vec(&body).unwrap());
        assert!(result.proposed_candidates.is_empty());
        assert_eq!(result.diagnostics[0]["code"], "advisory_malformed_result");
    }
}
#[tokio::test]
async fn only_admission_sets_category_and_deferred_candidates_remain_resurfaceable() {
    let state = inject(state().await, Arc::new(StubTransport::default()));
    configure(&state).await;
    seed(&state, A, "active", None).await;
    seed(&state, B, "active", None).await;
    let before = ubu_store::queries::get_current_state(state.inner().store.pool(), A)
        .await
        .unwrap()
        .unwrap();
    let response = run(&state, None).await;
    assert_eq!(
        ubu_store::queries::get_current_state(state.inner().store.pool(), A)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    let first = response["candidate_ids"][0].as_str().unwrap();
    let second = response["candidate_ids"][1].as_str().unwrap();
    action(&state, first, "defer", 1).await;
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    assert_eq!(queue["deferred_candidates"].as_array().unwrap().len(), 1);
    assert_eq!(queue["target_titles"][A], "Synthetic lunar teapot 0");
    action(&state, first, "resurface", 2).await;
    let admitted = action(&state, first, "admit", 3).await;
    assert_eq!(admitted["task"]["category_tag"], "work");
    assert!(admitted["task"]["tags"]
        .as_array()
        .unwrap()
        .contains(&json!("work")));
    let before = ubu_store::queries::get_current_state(state.inner().store.pool(), B)
        .await
        .unwrap();
    action(&state, second, "reject", 1).await;
    assert_eq!(
        ubu_store::queries::get_current_state(state.inner().store.pool(), B)
            .await
            .unwrap(),
        before
    );
}
#[tokio::test]
async fn rejected_proposal_does_not_return_on_a_second_run() {
    let state = inject(state().await, Arc::new(StubTransport::default()));
    configure(&state).await;
    seed(&state, A, "active", None).await;
    let first = run(&state, None).await;
    let id = first["candidate_ids"][0].as_str().unwrap();
    action(&state, id, "reject", 1).await;
    let second = run(&state, None).await;
    assert_eq!(second["selected"], first["selected"]);
    assert_eq!(second["candidates_enqueued"], 0);
    assert_eq!(second["report"]["candidates_suppressed"], 1);
    assert_eq!(count(&state).await, 1);
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    assert_eq!(queue["candidates"], json!([]));
    println!(
        "P1B45_TEST9={}",
        json!({"first_enqueued":first["candidates_enqueued"],"second_enqueued":second["candidates_enqueued"],"suppressed":second["report"]["candidates_suppressed"],"candidate_rows":count(&state).await,"queue":queue["candidates"]})
    );
}
#[tokio::test]
async fn real_transport_is_structurally_absent_from_test_configuration() {
    let main = include_str!("../src/main.rs");
    let lib = include_str!("../src/lib.rs");
    let services = include_str!("../src/services/mod.rs");
    assert!(main.starts_with("#[cfg(not(test))]\nmod ollama_transport;"));
    assert!(!lib.contains("ollama_transport"));
    assert!(!services.contains("ollama_transport"));
    assert!(
        main.contains("#[cfg(not(test))]\n    let state = state.with_advisory_transport_factory")
    );
    let state = state().await;
    assert!(state.advisory_transport_factory().is_none());
    configure(&state).await;
    seed(&state, A, "active", None).await;
    let response = run(&state, None).await;
    assert_eq!(
        response["diagnostics"][0]["code"],
        "advisory_transport_unavailable"
    );
    assert_eq!(count(&state).await, 0);
    println!("P1B45_TEST10=live module and installation are binary-only under cfg(not(test)); no library/service export; configured in-memory state has no factory; Run returns advisory_transport_unavailable and 0 candidate rows");
}
