use std::collections::{BTreeMap, BTreeSet};

use super::orchestrator::config::ServerConfig;
use super::orchestrator::services::advisory_service::run_advisory;
use super::orchestrator::state::AppState;
use ubu_core::worker::local_advisory::{
    AdvisoryCapability, AdvisoryTransport, ComputeBudget, LocalAdvisoryResult,
    LocalAdvisoryResultStatus, LocalAdvisorySubmission, ProviderConfig,
};
use ubu_core::worker::WorkerAuthority;
use ubu_core::{AuthoritySource, DeviceId, UbuId, UbuTimestamp};

pub(crate) const CANDIDATE: &str = r#"{
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
pub(crate) struct StubTransport {
    pub(crate) result: LocalAdvisoryResult,
}

impl AdvisoryTransport for StubTransport {
    fn submit(
        &self,
        _submission: &LocalAdvisorySubmission,
    ) -> ubu_core::Result<LocalAdvisoryResult> {
        Ok(self.result.clone())
    }
}

pub(crate) fn authority(granted: bool) -> WorkerAuthority {
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

pub(crate) fn submission(
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

pub(crate) fn result(
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

pub(crate) async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env()).await.unwrap()
}

pub(crate) fn proposal() -> ubu_core::AdvisoryCandidate {
    let mut candidate: ubu_core::AdvisoryCandidate = serde_json::from_str(CANDIDATE).unwrap();
    candidate.suppression_key = None;
    candidate
}

pub(crate) async fn ingest(state: &AppState, candidate: ubu_core::AdvisoryCandidate) {
    let mut authority = authority(true);
    authority
        .granted
        .insert(AdvisoryCapability::ProposeCandidate(
            candidate.candidate_kind,
        ));
    let submission = submission(authority, true);
    let transport = StubTransport {
        result: result(&submission, LocalAdvisoryResultStatus::Ok, vec![candidate]),
    };
    let report = run_advisory(state, submission, &transport).await.unwrap();
    assert_eq!(report.candidates_stored, 1, "{report:?}");
}

pub(crate) async fn seed_task(
    state: &AppState,
    tags: Vec<&str>,
) -> ubu_store::models::object_record::ObjectRecord {
    let candidate = proposal();
    let id = candidate.target_refs[0].id.clone();
    let now = UbuTimestamp::now_utc();
    let envelope = state
        .envelope_for(
            [(id.clone(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    ubu_store::api::admission::admit_object(state.inner().store.pool(), &envelope,
        ubu_store::models::object_record::NewObjectRecord {
            id: id.to_string(), object_type: "Task".into(), version: 1, status: "active".into(),
            compartment_label: "default".into(),
            payload: serde_json::json!({"id":id, "title":"Review target", "status":"active", "tags":tags,
                "provenance":{"created_at":now,"authority_source":"user"}}),
            created_at: now.to_string(), updated_at: now.to_string(),
        }).await.unwrap()
}
