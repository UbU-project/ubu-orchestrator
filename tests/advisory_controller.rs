use std::collections::{BTreeMap, BTreeSet};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use ubu_core::worker::local_advisory::{
    AdvisoryCapability, AdvisoryTransport, ComputeBudget, LocalAdvisoryResult,
    LocalAdvisoryResultStatus, LocalAdvisorySubmission, ProviderConfig,
};
use ubu_core::worker::WorkerAuthority;
use ubu_core::{AuthoritySource, DeviceId, UbuId, UbuTimestamp};
use ubu_orchestrator::api::advisory::AdvisoryQueueResponse;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::router::build_router;
use ubu_orchestrator::services::advisory_service::run_advisory;
use ubu_orchestrator::state::AppState;

const CANDIDATE: &str = r#"{
  "advisory_candidate_id": "advcand_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f",
  "schema_version": "1.0", "candidate_kind": "tag", "lifecycle_state": "proposed", "version": 1,
  "target_refs": [{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f","object_type":"Task"}],
  "normalized_proposal": {"operation":"add_tag","tag":"focus"},
  "payload": {"kind":"inline","value":{"rationale":"focus","tag":"focus"}},
  "evidence_refs": ["source:test"], "confidence": 0.8,
  "field_provenance": {"tag":"test"}, "proposed_at":"2026-09-19T09:00:00Z",
  "effective_time":"2026-09-19T09:00:00Z",
  "proposing_actor":{"model_or_tool_name":"test","version":"1","prompt_template_digest":"sha256:test"},
  "origin_device_id":"dev_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f",
  "execution_context":{"context_label":"advisory","backend_id":"test","provider_id":"test"},
  "idempotency_key":"proposal-tag-001", "suppression_key":"tag:focus:test", "compartment_ids":["comp_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f"],
  "review_label":{"kind":"label","value":"Personal"}, "disclosure_policy":"compartment_only", "retention_policy":"retain", "review_order":1, "links":{}
}"#;

#[derive(Clone)]
struct StubTransport {
    result: LocalAdvisoryResult,
}

impl AdvisoryTransport for StubTransport {
    fn submit(
        &self,
        _submission: &LocalAdvisorySubmission,
    ) -> ubu_core::Result<LocalAdvisoryResult> {
        Ok(self.result.clone())
    }
}

fn authority(granted: bool) -> WorkerAuthority {
    let mut capabilities = BTreeSet::new();
    if granted {
        capabilities.insert(AdvisoryCapability::ProposeCandidate(
            ubu_core::CandidateKind::Tag,
        ));
    } else {
        capabilities.insert(AdvisoryCapability::EmitDiagnostics);
    }
    WorkerAuthority {
        worker_id: UbuId::parse("worker_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f").unwrap(),
        authority_source: AuthoritySource::AutomationWorker,
        granted: capabilities,
        deadline: None,
    }
}

fn submission(
    authority: WorkerAuthority,
    partial_results_allowed: bool,
) -> LocalAdvisorySubmission {
    LocalAdvisorySubmission {
        submission_id: "submission-test".into(),
        authority,
        payload: serde_json::json!({"text":"test"}),
        expected_result_schema: "local-advisory-result".into(),
        timeout_ms: 1,
        compute_budget: ComputeBudget {
            max_cpu_ms: 1,
            max_memory_bytes: 1,
        },
        result_size_limit_bytes: 100_000,
        partial_results_allowed,
        causal_parents: vec![],
        observed_policy_versions: BTreeMap::new(),
        input_digests: BTreeMap::new(),
        provider_config: ProviderConfig {
            provider_name: "local".into(),
            provider_version: "1".into(),
            model_name: "test".into(),
            model_version: "1".into(),
            prompt_template_digest: None,
        },
        origin_device_id: DeviceId::parse("dev_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f").unwrap(),
        execution_context: None,
        submitted_at: UbuTimestamp::parse("2026-09-19T09:00:00Z").unwrap(),
    }
}

fn result(
    submission: &LocalAdvisorySubmission,
    status: LocalAdvisoryResultStatus,
    candidates: Vec<ubu_core::AdvisoryCandidate>,
) -> LocalAdvisoryResult {
    LocalAdvisoryResult {
        submission_id: submission.submission_id.clone(),
        authority: submission.authority.clone(),
        provider_config: submission.provider_config.clone(),
        observed_policy_versions: submission.observed_policy_versions.clone(),
        input_digests: submission.input_digests.clone(),
        status,
        artifacts: vec![],
        proposed_candidates: candidates,
        diagnostics: vec![serde_json::json!({"code":"stub_diagnostic"})],
        telemetry: vec![],
        deletion_confirmed: None,
    }
}

async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env()).await.unwrap()
}

#[tokio::test]
async fn happy_path_stores_candidates_and_queue_labels_candidate_state() {
    let state = state().await;
    let submission = submission(authority(true), true);
    let first: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    let mut second_json: serde_json::Value = serde_json::from_str(CANDIDATE).unwrap();
    second_json["advisory_candidate_id"] =
        serde_json::json!("advcand_118f3c8e9b2a7c4d8f1e2a3b4c5d6e7f");
    second_json["idempotency_key"] = serde_json::json!("proposal-tag-002");
    let second: ubu_core::AdvisoryCandidate = serde_json::from_value(second_json).unwrap();
    let transport = StubTransport {
        result: result(
            &submission,
            LocalAdvisoryResultStatus::Ok,
            vec![first, second],
        ),
    };

    let report = run_advisory(&state, submission, &transport).await.unwrap();
    assert_eq!(report.candidates_stored, 2);

    let response = build_router(state.clone())
        .oneshot(Request::get("/advisory/queue").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let queue: AdvisoryQueueResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(queue.state_category, "candidate_state");
    assert_eq!(queue.candidates.len(), 2);

    let response = build_router(state)
        .oneshot(
            Request::get("/advisory/candidate/advcand_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let candidate: ubu_orchestrator::api::advisory::AdvisoryCandidateResponse =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(candidate.state_category, "candidate_state");
}

#[tokio::test]
async fn failed_or_ungranted_results_store_nothing() {
    let state = state().await;
    let denied_submission = submission(authority(false), true);
    let candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    let denied_transport = StubTransport {
        result: result(
            &denied_submission,
            LocalAdvisoryResultStatus::Ok,
            vec![candidate],
        ),
    };
    let denied_report = run_advisory(&state, denied_submission, &denied_transport)
        .await
        .unwrap();
    assert_eq!(denied_report.candidates_stored, 0);
    assert!(denied_report.validation_error.is_some());

    for status in [
        LocalAdvisoryResultStatus::Rejected,
        LocalAdvisoryResultStatus::Timeout,
        LocalAdvisoryResultStatus::WorkerError,
        LocalAdvisoryResultStatus::MalformedResult,
        LocalAdvisoryResultStatus::Cancelled,
    ] {
        let failed_submission = submission(authority(true), true);
        let candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
        let failed_transport = StubTransport {
            result: result(&failed_submission, status, vec![candidate]),
        };
        let failed_report = run_advisory(&state, failed_submission, &failed_transport)
            .await
            .unwrap();
        assert_eq!(failed_report.candidates_stored, 0);
        assert!(!failed_report.diagnostics.is_empty());
    }
    let response = build_router(state)
        .oneshot(Request::get("/advisory/queue").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let queue: AdvisoryQueueResponse = serde_json::from_slice(&body).unwrap();
    assert!(queue.candidates.is_empty());
}

#[tokio::test]
async fn partial_results_follow_submission_permission_and_non_proposed_is_rejected() {
    let state = state().await;
    let allowed = submission(authority(true), true);
    let candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    let transport = StubTransport {
        result: result(
            &allowed,
            LocalAdvisoryResultStatus::Partial,
            vec![candidate],
        ),
    };
    assert_eq!(
        run_advisory(&state, allowed, &transport)
            .await
            .unwrap()
            .candidates_stored,
        1
    );

    let disallowed = submission(authority(true), false);
    let candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    let transport = StubTransport {
        result: result(
            &disallowed,
            LocalAdvisoryResultStatus::Partial,
            vec![candidate],
        ),
    };
    let report = run_advisory(&state, disallowed, &transport).await.unwrap();
    assert_eq!(report.candidates_stored, 0);
    assert!(report.validation_error.is_some());

    let non_proposed = submission(authority(true), true);
    let mut json: serde_json::Value = serde_json::from_str(CANDIDATE).unwrap();
    json["lifecycle_state"] = serde_json::json!("admitted");
    let candidate: ubu_core::AdvisoryCandidate = serde_json::from_value(json).unwrap();
    let transport = StubTransport {
        result: result(
            &non_proposed,
            LocalAdvisoryResultStatus::Ok,
            vec![candidate],
        ),
    };
    let report = run_advisory(&state, non_proposed, &transport)
        .await
        .unwrap();
    assert_eq!(report.candidates_stored, 0);
    assert_eq!(report.candidates_rejected, 1);
}

#[tokio::test]
async fn retry_with_same_candidate_is_idempotent() {
    let state = state().await;
    let submission = submission(authority(true), true);
    let candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    let transport = StubTransport {
        result: result(&submission, LocalAdvisoryResultStatus::Ok, vec![candidate]),
    };
    assert_eq!(
        run_advisory(&state, submission.clone(), &transport)
            .await
            .unwrap()
            .candidates_stored,
        1
    );
    assert_eq!(
        run_advisory(&state, submission, &transport)
            .await
            .unwrap()
            .candidates_stored,
        1
    );
}
