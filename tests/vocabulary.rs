//! Invented teapot fixtures only. Every submission uses StubTransport, never HTTP.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::{request, seed, task, A, B};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{
    services::{advisory_wire, precondition_advisor, vocabulary},
    state::AppState,
};
const FACT: &str = "facts.synthetic.teapot_ready";
const NUMBER: &str = "numeric_values.synthetic.teapot_charge";
struct StubTransport {
    answers: Mutex<Vec<Value>>,
    submissions: Mutex<Vec<LocalAdvisorySubmission>>,
    prior: bool,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(sub.clone());
        let answer = self.answers.lock().unwrap().remove(0);
        let bytes =
            serde_json::to_vec(&json!({"done":true,"response":answer.to_string()})).unwrap();
        let mut result = advisory_wire::interpret(sub, 200, &bytes);
        if self.prior {
            result
                .diagnostics
                .push(json!({"code":"synthetic_prior","message":"Synthetic prior diagnostic"}));
        }
        Ok(result)
    }
}
fn proposal(id: &str, target: &str) -> Value {
    json!({"id":id,"target":target})
}
fn answer(proposals: Vec<Value>) -> Value {
    json!({"proposals":proposals})
}
async fn bare() -> AppState {
    AppState::in_memory(ubu_orchestrator::config::ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(ubu_orchestrator::planning_time::FixedClock(
            ubu_core::UbuTimestamp::parse(fixture::NOW).unwrap(),
        ))
}
async fn configure(state: &AppState) {
    for (name, value) in [
        ("advisory.model", "synthetic-model:1"),
        ("advisory.endpoint", "http://127.0.0.1:11434"),
    ] {
        ubu_orchestrator::services::setting_authoring::put(state, name, json!(value))
            .await
            .unwrap();
    }
}
async fn ready(answers: Vec<Value>) -> (AppState, Arc<StubTransport>) {
    let stub = Arc::new(StubTransport {
        answers: Mutex::new(answers),
        submissions: Mutex::new(vec![]),
        prior: true,
    });
    let transport = stub.clone();
    let state = bare()
        .await
        .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
    configure(&state).await;
    seed(&state, A, "active", json!({})).await;
    (state, stub)
}
async fn run(state: &AppState) -> Value {
    let (status,body)=request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"vocabulary","limit":25})).await;
    assert_eq!(status, 200, "{body}");
    body
}
async fn admit(
    state: &AppState,
    id: &str,
    version: u64,
    value: Option<Value>,
) -> (axum::http::StatusCode, Value) {
    let mut body = json!({"observed_version":version});
    if let Some(value) = value {
        body["value"] = value;
    }
    request(
        state,
        "POST",
        &format!("/advisory/candidate/{id}/admit"),
        body,
    )
    .await
}
#[tokio::test]
async fn title_only_cold_start_proposes_without_writing_canonical_state_or_sending_values() {
    let (state, stub) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    let before = fixture::canonical_rows(&state).await;
    let body = run(&state).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["candidates_enqueued"], 1, "{body}");
    assert_eq!(fixture::canonical_rows(&state).await, before);
    let sub = stub.submissions.lock().unwrap()[0].clone();
    assert_eq!(sub.payload["targets"], json!([]));
    assert_eq!(sub.payload["tasks"][0].as_object().unwrap().len(), 2);
    let wire = advisory_wire::request_body(&sub).unwrap();
    assert_eq!(wire["format"]["properties"]["proposals"]["maxItems"], 3);
    assert_eq!(
        wire["format"]["properties"]["proposals"]["items"]["properties"]["target"]["pattern"],
        vocabulary::TARGET_PATTERN
    );
    assert_eq!(
        wire["format"]["properties"]["proposals"]["items"]["properties"]["target"]["maxLength"],
        128
    );
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    let candidate = &queue["candidates"][0]["candidate"];
    assert_eq!(
        candidate["normalized_proposal"],
        json!({"operation":"record_universe_target","target":FACT})
    );
    assert_eq!(candidate["target_refs"][0]["id"], A);
    assert!(candidate.get("suppression_key").is_none());
}
#[tokio::test]
async fn distinct_refusals_preserve_prior_diagnostics_and_never_echo_model_values() {
    let cases = [
        ("facts.affect.energy", vocabulary::Refusal::Reserved),
        (
            "set_memberships.synthetic.ready",
            vocabulary::Refusal::Collection,
        ),
        ("facts.synthetic..ready", vocabulary::Refusal::Grammar),
    ];
    for (target, reason) in cases {
        let (state, _) = ready(vec![answer(vec![proposal(A, target)])]).await;
        let body = run(&state).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["candidates_enqueued"], 0);
        assert_eq!(
            body["diagnostics"],
            json!([{"code":"synthetic_prior","message":"Synthetic prior diagnostic"},vocabulary::refusal(Some(A),reason)])
        );
    }
    let mut invalid = proposal(A, FACT);
    invalid["value"] = json!("synthetic-untrusted-value-must-not-survive");
    let (state, _) = ready(vec![answer(vec![invalid])]).await;
    let body = run(&state).await;
    assert_eq!(body["candidates_enqueued"], 0);
    assert!(!body
        .to_string()
        .contains("synthetic-untrusted-value-must-not-survive"));
    assert_eq!(
        body["diagnostics"][0],
        vocabulary::refusal(Some(A), vocabulary::Refusal::NameOnly)
    );
}
#[test]
fn name_validator_has_separate_length_grammar_collection_and_all_five_reserved_reasons() {
    let empty = Default::default();
    assert_eq!(
        vocabulary::validate_name(&format!("facts.{}", "a".repeat(123)), &empty),
        Err(vocabulary::Refusal::Length)
    );
    for name in ["facts.", "facts.a..b", "facts.a b", "facts.é"] {
        assert_eq!(
            vocabulary::validate_name(name, &empty),
            Err(vocabulary::Refusal::Grammar)
        );
    }
    for name in [
        "facts",
        "numeric_values",
        "set_memberships",
        "event_markers",
        "affect",
    ] {
        assert_eq!(
            vocabulary::validate_name(&format!("facts.{name}.x"), &empty),
            Err(vocabulary::Refusal::Reserved)
        );
    }
    for name in [
        "facts.fact.teapot",
        "facts.synthetic.affect.x",
        "numeric_values.synthetic-charge",
    ] {
        assert!(vocabulary::validate_name(name, &empty).is_ok());
    }
}
#[tokio::test]
async fn existing_target_is_refused_without_sending_its_value() {
    let (state, stub) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_fact","target":FACT,"payload":"synthetic-private-observation-sentinel"}]})).await;
    let body = run(&state).await;
    assert_eq!(body["candidates_enqueued"], 0);
    assert_eq!(
        body["diagnostics"][1],
        vocabulary::refusal(Some(A), vocabulary::Refusal::Existing)
    );
    let sub = stub.submissions.lock().unwrap()[0].clone();
    assert_eq!(sub.payload["targets"], json!([FACT]));
    assert!(!advisory_wire::request_body(&sub)
        .unwrap()
        .to_string()
        .contains("synthetic-private-observation-sentinel"));
}
#[tokio::test]
async fn bad_middle_of_three_costs_only_one_candidate_and_keeps_ok_and_prior_diagnostics() {
    let (state, _) = ready(vec![answer(vec![
        proposal(A, FACT),
        proposal(A, "facts.affect.energy"),
        proposal(A, NUMBER),
    ])])
    .await;
    let body = run(&state).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["candidates_enqueued"], 2, "{body}");
    assert_eq!(body["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["diagnostics"][1],
        vocabulary::refusal(Some(A), vocabulary::Refusal::Reserved)
    );
}
#[tokio::test]
async fn four_wire_proposals_refuse_whole_without_canonical_or_candidate_writes() {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT); 4])]).await;
    let before = fixture::canonical_rows(&state).await;
    let body = run(&state).await;
    assert_eq!(body["status"], "malformed_result");
    assert_eq!(body["candidates_enqueued"], 0);
    assert_eq!(fixture::count(&state, "advisory_candidates").await, 0);
    assert_eq!(fixture::canonical_rows(&state).await, before);
}
async fn seed_queue(state: &AppState, kind: &str, lifecycle: &str, total: usize) {
    for n in 0..total {
        sqlx::query("INSERT INTO advisory_candidates (advisory_candidate_id,candidate_kind,lifecycle_state,version,payload_json,created_at,updated_at) VALUES (?,?,?,1,'{}',?,?)")
        .bind(format!("advcand_018f3c8e9b2a7c4d8f1e2a3b{:08x}",n)).bind(kind).bind(lifecycle).bind(fixture::NOW).bind(fixture::NOW).execute(state.inner().store.pool()).await.unwrap();
    }
}
#[tokio::test]
async fn ten_awaiting_own_kind_blocks_before_transport_construction() {
    let (state, stub) = ready(vec![]).await;
    seed_queue(&state, "universe_target", "resurfaced", 10).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let transport = stub.clone();
    let state = state.with_advisory_transport_factory(Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        transport.clone()
    }));
    let body = run(&state).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["candidates_enqueued"], 0);
    assert_eq!(
        body["diagnostics"],
        json!([{"code":"vocabulary_queue_full","message":"10 target-name candidates are waiting in Review; review, defer or reject them before asking for more. No model was asked."}])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn full_precondition_or_deferred_queue_and_nine_own_candidates_allow_vocabulary() {
    for (kind, lifecycle, total) in [
        ("precondition", "proposed", 10),
        ("universe_target", "deferred", 10),
        ("universe_target", "proposed", 9),
    ] {
        let (state, stub) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
        seed_queue(&state, kind, lifecycle, total).await;
        let body = run(&state).await;
        assert_eq!(body["candidates_enqueued"], 1, "{body}");
        assert_eq!(stub.submissions.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn no_value_refuses_without_seed_and_explicit_false_asserts_then_enlarges_precondition_context(
) {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    let before = fixture::canonical_rows(&state).await;
    let (status, error) = admit(&state, id, 1, None).await;
    assert_eq!(status, 400, "{error}");
    assert!(fixture::diagnostic(&error, "vocabulary_value_required"));
    assert_eq!(fixture::canonical_rows(&state).await, before);
    let task_before = task(&state, A).await;
    let (status, admitted) = admit(&state, id, 1, Some(json!(false))).await;
    assert_eq!(status, 200, "{admitted}");
    let mut raw_task = task_before.clone();
    raw_task.as_object_mut().unwrap().remove("__status");
    raw_task.as_object_mut().unwrap().remove("__version");
    assert_eq!(admitted["task"], raw_task);
    assert_eq!(task(&state, A).await, task_before);
    let (world, version) = ubu_orchestrator::services::universe_state::read(&state)
        .await
        .unwrap();
    assert_eq!(version, Some(1));
    assert_eq!(world.facts["synthetic.teapot_ready"], false);
    assert_eq!(
        world.fact_provenance[FACT].kind,
        ubu_core::core::ProvenanceKind::Asserted
    );
    let (context, _) = precondition_advisor::select(&state, 25).await.unwrap();
    assert!(context.targets.contains(&FACT.into()));
    let sub = precondition_advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    assert_eq!(sub.payload["targets"], json!([FACT]));
    assert!(sub.payload.get("facts").is_none());
    let decisions = ubu_store::api::review::list_candidate_decision_events(
        state.inner().store.pool(),
        &ubu_core::AdvisoryCandidateId::parse(id).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        decisions[0].resulting_object_id.as_deref(),
        Some(world.id.as_str())
    );
}
#[tokio::test]
async fn numeric_invalid_value_refuses_then_explicit_zero_asserts_without_touching_task() {
    let (state, _) = ready(vec![answer(vec![proposal(A, NUMBER)])]).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    let before = fixture::canonical_rows(&state).await;
    let (status, error) = admit(&state, id, 1, Some(json!("not a number"))).await;
    assert_eq!(status, 400, "{error}");
    assert!(fixture::diagnostic(&error, "universe_mutation_invalid"));
    assert_eq!(fixture::canonical_rows(&state).await, before);
    assert_eq!(admit(&state, id, 1, Some(json!(0))).await.0, 200);
    let (world, _) = ubu_orchestrator::services::universe_state::read(&state)
        .await
        .unwrap();
    assert_eq!(world.numeric_values["synthetic.teapot_charge"], 0.0);
    assert_eq!(
        world.fact_provenance[NUMBER].kind,
        ubu_core::core::ProvenanceKind::Asserted
    );
}
#[tokio::test]
async fn existing_value_written_after_proposal_is_never_overwritten_by_admission() {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    let (status,_)=request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_fact","target":FACT,"payload":true}]})).await;
    assert_eq!(status, 200);
    let before = fixture::canonical_rows(&state).await;
    let (status, error) = admit(&state, id, 1, Some(json!(false))).await;
    assert_eq!(status, 400, "{error}");
    assert!(fixture::diagnostic(&error, "vocabulary_admission_refused"));
    assert_eq!(fixture::canonical_rows(&state).await, before);
}
#[tokio::test]
async fn deferred_and_stale_candidate_admission_is_atomic_without_an_empty_seed() {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    assert_eq!(fixture::action(&state, id, "defer", 1).await.0, 200);
    let before = fixture::canonical_rows(&state).await;
    assert_eq!(admit(&state, id, 2, Some(json!(true))).await.0, 409);
    assert_eq!(fixture::canonical_rows(&state).await, before);
    assert_eq!(fixture::action(&state, id, "resurface", 2).await.0, 200);
    assert_eq!(admit(&state, id, 1, Some(json!(true))).await.0, 409);
    assert_eq!(admit(&state, id, 3, Some(json!(true))).await.0, 200);
    let before = fixture::canonical_rows(&state).await;
    assert_ne!(admit(&state, id, 4, Some(json!(true))).await.0, 200);
    assert_eq!(fixture::canonical_rows(&state).await, before);
}
#[tokio::test]
async fn rejection_suppresses_same_name_and_task_but_not_other_name_or_task() {
    let (state, _) = ready(vec![
        answer(vec![proposal(A, FACT)]),
        answer(vec![
            proposal(A, FACT),
            proposal(A, NUMBER),
            proposal(B, FACT),
        ]),
    ])
    .await;
    seed(&state, B, "active", json!({})).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    assert_eq!(fixture::action(&state, id, "reject", 1).await.0, 200);
    let body = run(&state).await;
    assert_eq!(body["candidates_enqueued"], 2, "{body}");
    assert!(fixture::diagnostic(&body, "advisory_proposal_suppressed"));
}
#[tokio::test]
async fn aggregated_refusals_name_three_then_count_and_keep_prior_diagnostics() {
    let (state, stub) = ready(vec![
        answer(vec![proposal(A, "facts.affect.x"); 3]),
        answer(vec![proposal(A, "facts.affect.x"); 3]),
    ])
    .await;
    let (context, _) = precondition_advisor::select(&state, 25).await.unwrap();
    let sub = vocabulary::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let mut result = stub.submit(&sub).unwrap();
    result
        .proposed_candidates
        .extend(stub.submit(&sub).unwrap().proposed_candidates);
    vocabulary::vet_result(&state, &sub, &mut result)
        .await
        .unwrap();
    assert!(result.proposed_candidates.is_empty());
    assert_eq!(result.diagnostics.len(), 5);
    assert_eq!(result.diagnostics[0]["code"], "synthetic_prior");
    assert_eq!(result.diagnostics[4]["message"],"3 more Tasks had refused target-name proposals; no candidates were enqueued for those Tasks. The rest of the run stands.");
}
#[test]
fn explicit_json_null_is_distinct_from_an_omitted_operator_value() {
    use ubu_orchestrator::api::advisory::AdmitRequest;
    assert!(
        serde_json::from_value::<AdmitRequest>(json!({"observed_version":1}))
            .unwrap()
            .value
            .is_none()
    );
    assert_eq!(
        serde_json::from_value::<AdmitRequest>(json!({"observed_version":1,"value":null}))
            .unwrap()
            .value,
        Some(Value::Null)
    );
}

#[tokio::test]
async fn injected_proposed_value_and_unselected_task_are_refused_at_controller_boundary() {
    let (state, stub) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    seed(&state, B, "active", json!({})).await;
    let (mut context, _) = precondition_advisor::select(&state, 25).await.unwrap();
    context.tasks.retain(|t| t.id == A);
    let sub = vocabulary::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let result = stub.submit(&sub).unwrap();
    let mut injected = result.clone();
    injected.proposed_candidates[0].normalized_proposal["value"] =
        json!("synthetic-injected-value");
    vocabulary::vet_result(&state, &sub, &mut injected)
        .await
        .unwrap();
    assert!(injected.proposed_candidates.is_empty());
    assert!(!serde_json::to_string(&injected)
        .unwrap()
        .contains("synthetic-injected-value"));
    let mut unselected = result;
    unselected.proposed_candidates[0].target_refs[0].id = ubu_core::UbuId::parse(B).unwrap();
    vocabulary::vet_result(&state, &sub, &mut unselected)
        .await
        .unwrap();
    assert!(unselected.proposed_candidates.is_empty());
    assert_eq!(
        unselected.diagnostics[1],
        vocabulary::refusal(Some(B), vocabulary::Refusal::TaskReference)
    );
}
#[tokio::test]
async fn inactive_evidence_refuses_admission_and_preserves_existing_world_label_and_values() {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    let (status,_)=request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_numeric","target":NUMBER,"payload":7}]})).await;
    assert_eq!(status, 200);
    sqlx::query("UPDATE objects SET status='moot', payload_json=json_set(payload_json,'$.status','moot') WHERE id=?").bind(A).execute(state.inner().store.pool()).await.unwrap();
    let before = fixture::canonical_rows(&state).await;
    let (status, error) = admit(&state, id, 1, Some(json!(true))).await;
    assert_eq!(status, 400, "{error}");
    assert!(fixture::diagnostic(&error, "vocabulary_admission_refused"));
    assert_eq!(fixture::canonical_rows(&state).await, before);
}
#[tokio::test]
async fn admission_to_existing_state_keeps_label_capture_time_summary_and_other_values() {
    let (state, _) = ready(vec![answer(vec![proposal(A, FACT)])]).await;
    request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_numeric","target":NUMBER,"payload":7}]})).await;
    let (old, version) = ubu_orchestrator::services::universe_state::read(&state)
        .await
        .unwrap();
    sqlx::query("UPDATE objects SET compartment_label='synthetic-existing-label' WHERE id=?")
        .bind(old.id.as_str())
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let body = run(&state).await;
    let id = body["candidate_ids"][0].as_str().unwrap();
    assert_eq!(admit(&state, id, 1, Some(Value::Null)).await.0, 200);
    let (world, new_version) = ubu_orchestrator::services::universe_state::read(&state)
        .await
        .unwrap();
    assert_eq!(world.numeric_values, old.numeric_values);
    assert_eq!(world.captured_at, old.captured_at);
    assert_eq!(world.source_summary, old.source_summary);
    assert_eq!(new_version, version.map(|v| v + 1));
    assert_eq!(world.facts["synthetic.teapot_ready"], Value::Null);
    let row = ubu_store::queries::get_current_state(state.inner().store.pool(), world.id.as_str())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.compartment_label, "synthetic-existing-label");
}

#[tokio::test]
async fn selection_reuses_title_or_notes_gate_and_does_not_ask_for_empty_or_occurrence_tasks() {
    let stub = Arc::new(StubTransport {
        answers: Mutex::new(vec![]),
        submissions: Mutex::new(vec![]),
        prior: false,
    });
    let transport = stub.clone();
    let state = bare()
        .await
        .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
    configure(&state).await;
    seed(&state, A, "active", json!({"title":"   "})).await;
    seed(&state, B, "active", json!({"occurrence":{"routine_objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01","local_date":"2026-09-29","key":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-29T07:00:00/static/t1"}})).await;
    let body = run(&state).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["selected"], json!([]));
    assert!(fixture::diagnostic(&body, "advisory_task_skipped"));
    assert!(fixture::diagnostic(&body, "vocabulary_no_task"));
    assert!(stub.submissions.lock().unwrap().is_empty());
}
