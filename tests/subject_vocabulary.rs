//! Invented teapot names only; every producer transport is StubTransport.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::{request, seed, A};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{
    config::ServerConfig,
    services::{
        advisory_wire, precondition_advisor, setting_authoring, subject_vocabulary, vocabulary,
    },
    state::AppState,
};
struct StubTransport {
    proposals: Value,
    submissions: Mutex<Vec<LocalAdvisorySubmission>>,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(sub.clone());
        Ok(advisory_wire::interpret(
            sub,
            200,
            &serde_json::to_vec(&json!({"done":true,"response":self.proposals.to_string()}))
                .unwrap(),
        ))
    }
}
async fn ready(proposals: Value) -> (AppState, Arc<StubTransport>) {
    let stub = Arc::new(StubTransport {
        proposals,
        submissions: Mutex::new(vec![]),
    });
    let transport = stub.clone();
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
    setting_authoring::put(&state, "universe.subject.teapot", json!(true))
        .await
        .unwrap();
    setting_authoring::put(&state, "advisory.model", json!("synthetic-subject-model"))
        .await
        .unwrap();
    setting_authoring::put(&state, "advisory.endpoint", json!("http://127.0.0.1:11434"))
        .await
        .unwrap();
    seed(&state, A, "active", json!({})).await;
    (state, stub)
}
async fn run(state: &AppState) -> Value {
    let(status,body)=request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"vocabulary","limit":25})).await;
    assert_eq!(status, 200, "{body}");
    body
}
#[tokio::test]
async fn schema_and_context_name_effective_subjects_minus_affect_without_observations() {
    let (state, stub) = ready(json!({"proposals":[]})).await;
    run(&state).await;
    let sub = stub.submissions.lock().unwrap()[0].clone();
    let body = advisory_wire::request_body(&sub).unwrap();
    assert_eq!(
        sub.payload["subjects"],
        json!(["github", "operator", "project", "relationship", "teapot"])
    );
    assert_eq!(
        body["format"]["properties"]["proposals"]["items"]["properties"]["target"],
        json!({"type":"string","pattern":r"^(facts|numeric_values)\.(github|operator|project|relationship|teapot)\.([A-Za-z0-9_-]+\.)*[a-z][a-z0-9]*(_[a-z0-9]+)*$","maxLength":128})
    );
    assert!(!body["system"]
        .as_str()
        .unwrap()
        .contains("Propose only facts or numeric_values targets"));
    assert!(sub.payload.get("facts").is_none());
    assert!(sub.payload.get("fact_provenance").is_none());
}
#[tokio::test]
async fn unknown_subject_and_missing_predicate_are_independent_refusals_and_survivor_stays_ok() {
    let(state,_)=ready(json!({"proposals":[{"id":A,"target":"facts.teapot.ready"},{"id":A,"target":"facts.unminted.ready"},{"id":A,"target":"facts.operator"}]})).await;
    let before = fixture::canonical_rows(&state).await;
    let body = run(&state).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["candidates_enqueued"], 1, "{body}");
    assert_eq!(fixture::canonical_rows(&state).await, before);
    assert_eq!(
        body["diagnostics"],
        json!([
            vocabulary::refusal(Some(A), vocabulary::Refusal::Subject),
            vocabulary::refusal(Some(A), vocabulary::Refusal::Grammar)
        ])
    );
}
#[tokio::test]
async fn schema_grammar_witnesses_and_controller_checks_preserve_reserved_and_existing_refusals() {
    let (state, _) = ready(json!({"proposals":[]})).await;
    let subjects = vocabulary::subjects(&state).await.unwrap();
    let empty = Default::default();
    for target in [
        "facts.teapot.ready",
        "numeric_values.operator.level",
        "facts.github.issue.14.pipeline_state",
        "facts.project.A-14.ready",
    ] {
        assert_eq!(
            vocabulary::validate_name(target, &empty, &subjects),
            Ok(()),
            "{target}"
        );
    }
    for (target, reason) in [
        ("facts.unminted.ready", vocabulary::Refusal::Subject),
        ("facts.operator", vocabulary::Refusal::Grammar),
        ("facts.operator.Upper_leaf", vocabulary::Refusal::Grammar),
        ("facts.affect.energy", vocabulary::Refusal::Reserved),
        ("facts.operator.a..ready", vocabulary::Refusal::Grammar),
        (
            "set_memberships.operator.labels",
            vocabulary::Refusal::Collection,
        ),
    ] {
        assert_eq!(
            vocabulary::validate_name(target, &empty, &subjects),
            Err(reason),
            "{target}"
        );
    }
    let existing = ["facts.teapot.ready".to_owned()].into_iter().collect();
    assert_eq!(
        vocabulary::validate_name("facts.teapot.ready", &existing, &subjects),
        Err(vocabulary::Refusal::Existing)
    );
    assert_eq!(
        subject_vocabulary::target_pattern(&subjects),
        r"^(facts|numeric_values)\.(github|operator|project|relationship|teapot)\.([A-Za-z0-9_-]+\.)*[a-z][a-z0-9]*(_[a-z0-9]+)*$"
    );
}
#[tokio::test]
async fn retirement_after_submission_refuses_enqueue_and_after_enqueue_refuses_admission() {
    let (state, stub) = ready(json!({"proposals":[{"id":A,"target":"facts.teapot.ready"}]})).await;
    let (context, _) = precondition_advisor::select(&state, 25).await.unwrap();
    let sub = vocabulary::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let mut result = stub.submit(&sub).unwrap();
    setting_authoring::delete(&state, "universe.subject.teapot")
        .await
        .unwrap();
    vocabulary::vet_result(&state, &sub, &mut result)
        .await
        .unwrap();
    assert!(result.proposed_candidates.is_empty());
    assert_eq!(
        result.diagnostics[0],
        vocabulary::refusal(Some(A), vocabulary::Refusal::Subject)
    );
    setting_authoring::put(&state, "universe.subject.teapot", json!(true))
        .await
        .unwrap();
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    setting_authoring::delete(&state, "universe.subject.teapot")
        .await
        .unwrap();
    let before = fixture::canonical_rows(&state).await;
    let (status, error) = request(
        &state,
        "POST",
        &format!("/advisory/candidate/{id}/admit"),
        json!({"observed_version":1,"value":true}),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(
        error["diagnostics"][0]["code"],
        "vocabulary_admission_refused"
    );
    assert_eq!(fixture::canonical_rows(&state).await, before);
}
#[tokio::test]
async fn precondition_schema_is_byte_identical_across_registry_changes_and_keeps_legacy_targets() {
    let (state, _) = ready(json!({"proposals":[]})).await;
    let mut context = precondition_advisor::Context {
        tasks: vec![precondition_advisor::DescribedTask {
            id: A.into(),
            title: "Synthetic legacy condition".into(),
            description: None,
        }],
        targets: vec!["facts.legacy_leaf".into()],
    };
    let first = precondition_advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let format = advisory_wire::request_body(&first).unwrap()["format"].to_string();
    setting_authoring::delete(&state, "universe.subject.teapot")
        .await
        .unwrap();
    context.targets.push("facts.retired_root.ready".into());
    let changed = precondition_advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    assert!(advisory_wire::request_body(&changed).unwrap()["format"]
        .to_string()
        .contains("facts.legacy_leaf"));
    context.targets.pop();
    let after = precondition_advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    assert_eq!(
        advisory_wire::request_body(&after).unwrap()["format"].to_string(),
        format
    );
}
