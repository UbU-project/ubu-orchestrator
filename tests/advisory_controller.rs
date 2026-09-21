use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use ubu_core::worker::local_advisory::LocalAdvisoryResultStatus;
use ubu_orchestrator::api::advisory::AdvisoryQueueResponse;
use ubu_orchestrator::router::build_router;
use ubu_orchestrator::services::advisory_service::run_advisory;

use ubu_orchestrator as orchestrator;
#[path = "support/advisory_fixture.rs"]
mod review_fixture;
use review_fixture::*;
use serde_json::{json, Value};
use ubu_orchestrator::state::AppState;
use ubu_store::api::review::{
    find_suppression_record, get_advisory_candidate, list_candidate_decision_events,
};

async fn action(state: &AppState, action: &str, body: Value) -> (StatusCode, Value) {
    let id = proposal().advisory_candidate_id;
    let response = build_router(state.clone())
        .oneshot(
            Request::post(format!("/advisory/candidate/{}/{action}", id.as_str()))
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
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

async fn review_snapshot(state: &AppState) -> Value {
    let pool = state.inner().store.pool();
    let candidate = proposal();
    let mut counts = Vec::new();
    for table in [
        "objects",
        "logs",
        "external_references",
        "plans",
        "calendars",
        "snapshots",
        "projection_previews",
        "projection_results",
        "worker_submissions",
        "mutation_envelopes",
        "suppression_records",
    ] {
        counts.push(
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    json!({"counts":counts,
        "task":ubu_store::queries::get_current_state(pool,candidate.target_refs[0].id.as_str()).await.unwrap(),
        "candidate":get_advisory_candidate(pool,&candidate.advisory_candidate_id).await.unwrap(),
        "events":list_candidate_decision_events(pool,&candidate.advisory_candidate_id).await.unwrap()})
}

#[tokio::test]
async fn admit_applies_tag_and_links_user_decision_to_task() {
    let state = state().await;
    let target = seed_task(&state, vec![]).await;
    ingest(&state, proposal()).await;
    let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state_category"], "candidate_state");
    assert_eq!(body["task"]["tags"], json!(["focus"]));
    assert_eq!(body["candidate"]["lifecycle_state"], "admitted");
    let snapshot = review_snapshot(&state).await;
    let stored: Value =
        serde_json::from_str(snapshot["task"]["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(stored, body["task"]);
    assert_eq!(snapshot["task"]["version"], 2);
    let events = list_candidate_decision_events(
        state.inner().store.pool(),
        &proposal().advisory_candidate_id,
    )
    .await
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].resulting_object_id.as_deref(),
        Some(target.id.as_str())
    );
    let envelope: ubu_core::MutationEnvelope =
        serde_json::from_str(&events[0].envelope_json).unwrap();
    assert_eq!(envelope.actor_identity_id, *state.actor_identity_id());
    assert_eq!(envelope.authority_source, ubu_core::AuthoritySource::User);
    assert_eq!(
        envelope
            .observed_versions
            .get(&proposal().target_refs[0].id),
        Some(&ubu_core::VersionRef::Version(1))
    );
    assert_eq!(body["candidate"]["links"]["admission_ref"], target.id);
}

#[tokio::test]
async fn already_present_tag_admits_once_without_suppression_and_double_submit_is_clean() {
    let state = state().await;
    let target = seed_task(&state, vec!["focus"]).await;
    ingest(&state, proposal()).await;
    let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["candidate"]["lifecycle_state"], "admitted");
    assert_eq!(
        body["task"],
        serde_json::from_str::<Value>(&target.payload_json).unwrap()
    );
    let before = review_snapshot(&state).await;
    assert_eq!(before["counts"][10], 0);
    let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["diagnostics"][0]["code"], "PreconditionFailed");
    assert_eq!(review_snapshot(&state).await, before);
}

#[tokio::test]
async fn reject_preserves_canonical_state_and_derives_readable_suppression() {
    let state = state().await;
    let task = seed_task(&state, vec![]).await;
    ingest(&state, proposal()).await;
    let before = review_snapshot(&state).await;
    let (status, body) = action(
        &state,
        "reject",
        json!({"observed_version":1,"reason":"Not useful","retention_policy":"purge_payload"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state_category"], "candidate_state");
    assert_eq!(body["candidate"]["lifecycle_state"], "rejected");
    let after = review_snapshot(&state).await;
    assert_eq!(before["task"], after["task"]);
    assert_eq!(
        &before["counts"].as_array().unwrap()[..9],
        &after["counts"].as_array().unwrap()[..9]
    );
    let key = body["candidate"]["suppression_key"].as_str().unwrap();
    assert_eq!(
        key,
        format!(
            r#"{{"candidate_kind":"tag","normalized_proposal":{{"operation":"add_tag","tag":"focus"}},"target_refs":[{{"id":"{}","object_type":"Task"}}]}}"#,
            task.id
        )
    );
    let suppression = find_suppression_record(state.inner().store.pool(), key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        suppression.rejection_reason_or_user_correction,
        "Not useful"
    );
    assert_eq!(
        suppression.retention_policy,
        ubu_core::RetentionPolicy::PurgePayload
    );
    assert_eq!(
        suppression.deciding_actor_identity_id,
        *state.actor_identity_id()
    );
    assert_eq!(
        suppression.authority_source,
        ubu_core::AuthoritySource::User
    );
    assert!(suppression
        .evidence_hashes_or_source_fingerprints
        .is_empty());
}

#[tokio::test]
async fn deferred_candidate_requires_resurfacing_before_admission() {
    let state = state().await;
    seed_task(&state, vec![]).await;
    ingest(&state, proposal()).await;
    let (status, body) = action(&state, "defer", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state_category"], "candidate_state");
    assert_eq!(body["candidate"]["lifecycle_state"], "deferred");
    let before = review_snapshot(&state).await;
    let (status, body) = action(&state, "admit", json!({"observed_version":2})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["diagnostics"][0]["code"], "InvalidCandidateTransition");
    assert_eq!(review_snapshot(&state).await, before);
    let (status, body) = action(
        &state,
        "resurface",
        json!({"observed_version":2,"trigger":"user_request"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state_category"], "candidate_state");
    assert_eq!(body["candidate"]["lifecycle_state"], "resurfaced");
    assert_eq!(
        body["candidate"]["links"]["resurface_trigger"],
        "user_request"
    );
    let (status, body) = action(&state, "admit", json!({"observed_version":3})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["candidate"]["lifecycle_state"], "admitted");
    assert_eq!(body["task"]["tags"], json!(["focus"]));
    let events = list_candidate_decision_events(
        state.inner().store.pool(),
        &proposal().advisory_candidate_id,
    )
    .await
    .unwrap();
    let envelopes: Vec<ubu_core::MutationEnvelope> = events
        .iter()
        .map(|e| serde_json::from_str(&e.envelope_json).unwrap())
        .collect();
    assert_eq!(envelopes.len(), 3);
    assert!(envelopes
        .iter()
        .all(|e| e.authority_source == ubu_core::AuthoritySource::User));
    assert!(envelopes[0].observed_versions.is_empty() && envelopes[1].observed_versions.is_empty());
    assert_ne!(envelopes[0].idempotency_key, envelopes[1].idempotency_key);
    assert_ne!(envelopes[1].idempotency_key, envelopes[2].idempotency_key);
}

#[tokio::test]
async fn unsupported_kind_or_operation_and_malformed_proposals_write_nothing() {
    for case in [
        "kind",
        "operation",
        "empty_tag",
        "missing_tag",
        "many_targets",
        "wrong_target_kind",
        "no_targets",
    ] {
        let state = state().await;
        seed_task(&state, vec![]).await;
        let mut candidate = proposal();
        match case {
            "kind" => candidate.candidate_kind = ubu_core::CandidateKind::Dependency,
            "operation" => candidate.normalized_proposal["operation"] = json!("remove_tag"),
            "empty_tag" => candidate.normalized_proposal["tag"] = json!(""),
            "missing_tag" => {
                candidate
                    .normalized_proposal
                    .as_object_mut()
                    .unwrap()
                    .remove("tag");
            }
            "many_targets" => candidate.target_refs.push(ubu_core::ObjectRef {
                id: ubu_core::UbuId::new(ubu_core::ObjectType::Task),
                object_type: ubu_core::ObjectType::Task,
            }),
            "wrong_target_kind" => {
                candidate.target_refs = vec![ubu_core::ObjectRef {
                    id: ubu_core::UbuId::new(ubu_core::ObjectType::Objective),
                    object_type: ubu_core::ObjectType::Objective,
                }]
            }
            _ => candidate.target_refs.clear(),
        }
        ingest(&state, candidate).await;
        let before = review_snapshot(&state).await;
        let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
        if matches!(case, "kind" | "operation") {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{case}: {body}");
            assert_eq!(body["diagnostics"][0]["code"], "UnsupportedProposal");
            assert!(body["error"].as_str().unwrap().contains(if case == "kind" {
                "Dependency"
            } else {
                "remove_tag"
            }));
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {body}");
        }
        assert_eq!(review_snapshot(&state).await, before, "{case}");
    }
}

#[tokio::test]
async fn redacted_payload_admits_from_normalized_proposal() {
    let state = state().await;
    seed_task(&state, vec![]).await;
    let mut candidate = proposal();
    candidate.payload = ubu_core::CandidatePayload::RedactedSummary("Contents withheld".into());
    ingest(&state, candidate).await;
    let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["task"]["tags"], json!(["focus"]));
    assert_eq!(body["candidate"]["lifecycle_state"], "admitted");
}

#[tokio::test]
async fn missing_target_returns_named_error_without_decision() {
    let state = state().await;
    ingest(&state, proposal()).await;
    let before = review_snapshot(&state).await;
    let (status, body) = action(&state, "admit", json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["diagnostics"][0]["code"], "TargetNotFound");
    assert_eq!(review_snapshot(&state).await, before);
}

#[test]
fn openapi_documents_all_review_routes_and_core_enum_values() {
    use utoipa::OpenApi;
    let api = serde_json::to_value(ubu_orchestrator::openapi::ApiDoc::openapi()).unwrap();
    for action in ["admit", "reject", "defer", "resurface"] {
        assert!(
            api["paths"][format!("/advisory/candidate/{{candidate_id}}/{action}")]["post"]
                .is_object()
        );
    }
    assert_eq!(
        api["components"]["schemas"]["RejectRequest"]["properties"]["retention_policy"]["enum"],
        json!(["retain", "purge_payload"])
    );
    assert!(
        api["components"]["schemas"]["ResurfaceRequest"]["properties"]["trigger"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("user_request"))
    );
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
