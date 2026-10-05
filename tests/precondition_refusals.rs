#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::core::{InstanceMode, UniverseState};
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{
    services::{advisory_wire, precondition_advisor as advisor},
    state::AppState,
};

struct StubTransport {
    trees: Vec<Value>,
    prior: bool,
    wrong_result: bool,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        let proposals: Vec<_> = sub.payload["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&self.trees)
            .map(|(task, tree)| json!({"id":task["id"],"precondition":tree}))
            .collect();
        let answer = if self.wrong_result {
            json!({"proposals":false})
        } else {
            json!({"proposals":proposals})
        };
        let mut result = advisory_wire::interpret(
            sub,
            200,
            &serde_json::to_vec(&json!({"done":true,"response":answer.to_string()})).unwrap(),
        );
        if self.prior {
            result.diagnostics.push(
                json!({"code":"synthetic_prior_diagnostic","message":"Synthetic prior diagnostic"}),
            );
        }
        Ok(result)
    }
}

fn id(n: usize) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d{:04x}", 0x6e70 + n)
}
async fn batch(trees: Vec<Value>, prior: bool, wrong_result: bool) -> AppState {
    let (state, _) = ready(tree()).await;
    fact(&state, 30.0).await;
    for n in 1..trees.len() {
        seed(&state, &id(n), "active", json!({})).await;
    }
    let stub = Arc::new(StubTransport {
        trees,
        prior,
        wrong_result,
    });
    state.with_advisory_transport_factory(Arc::new(move |_| stub.clone()))
}

#[tokio::test]
async fn a_bad_middle_proposal_costs_only_its_candidate_and_preserves_canonical_state() {
    let state = batch(
        vec![
            tree(),
            json!({"target":TARGET,"predicate":"equals"}),
            tree(),
        ],
        false,
        false,
    )
    .await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 2);
    assert_eq!(count(&state, "advisory_candidates").await, 2);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger + 2);
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(
        result["diagnostics"],
        json!([advisor::refusal_diagnostic(
            Some(B),
            &advisor::TreeRefusal::ExpectedRequired
        )])
    );
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    let ids: Vec<_> = queue["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["candidate"]["target_refs"][0]["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&A) && ids.contains(&id(2).as_str()) && !ids.contains(&B));
}

#[tokio::test]
async fn diagnostics_before_a_refusal_survive_including_missing_target_and_transport_diagnostics() {
    let state = batch(
        vec![
            json!({"target":"facts.synthetic.missing","predicate":"absent"}),
            json!({"target":TARGET,"predicate":"equals"}),
            tree(),
        ],
        true,
        false,
    )
    .await;
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 1);
    assert_eq!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "synthetic_prior_diagnostic",
            "precondition_missing_targets",
            "precondition_proposal_refused"
        ]
    );
}

#[tokio::test]
async fn all_refused_proposals_keep_ok_status_and_name_three_then_count_seven() {
    let state = batch(
        vec![
            json!({"target":TARGET,"predicate":"at_least","expected":"synthetic-private-expected"});
            10
        ],
        false,
        false,
    )
    .await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 0);
    assert_eq!(result["report"]["proposals"], json!([]));
    let diagnostics = result["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 4);
    for (n, diagnostic) in diagnostics.iter().take(3).enumerate() {
        assert_eq!(diagnostic["code"], "precondition_proposal_refused");
        assert!(diagnostic["message"].as_str().unwrap().contains(&id(n)));
    }
    assert_eq!(
        diagnostics[3],
        json!({"code":"precondition_proposal_refused","message":"7 more Tasks had unevaluable proposals; no candidates were enqueued for those Tasks. The rest of the run stands."})
    );
    assert!(
        !diagnostics[3].to_string().contains("task_")
            && !diagnostics[3].to_string().contains(TARGET)
    );
    assert!(!result.to_string().contains("synthetic-private-expected"));
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
}

#[tokio::test]
async fn a_response_that_is_not_a_proposal_result_remains_malformed() {
    let state = batch(vec![tree()], false, true).await;
    let before = canonical_rows(&state).await;
    let result = run(&state).await;
    assert_eq!(result["status"], "malformed_result");
    assert_eq!(result["candidates_enqueued"], 0);
    assert!(diagnostic(&result, "advisory_malformed_result"));
    assert!(!diagnostic(&result, "precondition_proposal_refused"));
    assert_eq!(canonical_rows(&state).await, before);
}

#[test]
fn each_reason_has_a_code_authored_diagnostic_with_no_model_value() {
    use advisor::TreeRefusal::*;
    for (reason, text) in [
        (BoundExceeded, "the tree exceeds 128 nodes or depth 16"),
        (
            InvalidGroup,
            "the tree must contain leaves or single-key boolean groups with non-empty arrays",
        ),
        (
            MissingLeafFields,
            "a leaf requires string target and predicate fields",
        ),
        (
            UnknownLeafFields,
            "a leaf has an unrecognised target, predicate or key",
        ),
        (
            ExpectedRequired,
            "this predicate requires an expected value",
        ),
        (ExpectedForbidden, "absent forbids an expected value"),
        (
            TreeDeserialization,
            "the tree cannot be decoded without losing fields",
        ),
        (
            NullExpectation,
            "a null expected value cannot be represented by this precondition",
        ),
        (
            ModeRefusal,
            "this instance mode does not permit an intrinsic-affect target",
        ),
        (
            EvaluatorRefusal("at_least expected value must be a finite number".into()),
            "at_least expected value must be a finite number",
        ),
        (
            InvalidTaskReference,
            "the proposal must reference exactly one Task",
        ),
    ] {
        assert_eq!(
            advisor::refusal_diagnostic(Some(A), &reason),
            json!({"code":"precondition_proposal_refused","message":format!("Task `{A}`: {text}. No candidate was enqueued for this Task; the rest of the run stands.")})
        );
    }
}

#[test]
fn validation_classifies_real_refusals_without_losing_evaluation_or_mode_guards() {
    use advisor::TreeRefusal::*;
    let mut state = UniverseState::new(
        ubu_core::UbuTimestamp::parse(NOW).unwrap(),
        "Synthetic reason test",
    );
    state
        .numeric_values
        .insert("synthetic.orbital_teapot_charge".into(), 30.0);
    let mut deep = tree();
    for _ in 0..17 {
        deep = json!({"all_of":[deep]});
    }
    for (raw, reason) in [
        (deep, BoundExceeded),
        (json!({"all_of":vec![tree();128]}), BoundExceeded),
        (json!({"all_of":[]}), InvalidGroup),
        (json!({"target":TARGET}), MissingLeafFields),
        (
            json!({"target":"facts.private prose","predicate":"equals","expected":"synthetic-private-expected"}),
            UnknownLeafFields,
        ),
        (
            json!({"target":TARGET,"predicate":"equals"}),
            ExpectedRequired,
        ),
        (
            json!({"target":TARGET,"predicate":"absent","expected":"synthetic-private-expected"}),
            ExpectedForbidden,
        ),
        (
            json!({"target":TARGET,"predicate":"equals","expected":null}),
            NullExpectation,
        ),
        (
            json!({"target":TARGET,"predicate":"at_least","expected":"synthetic-private-expected"}),
            EvaluatorRefusal("at_least expected value must be a finite number".into()),
        ),
        (
            json!({"target":"facts.synthetic.ready","predicate":"member_of","expected":true}),
            EvaluatorRefusal("member_of requires a set_memberships target".into()),
        ),
        (
            json!({"target":"facts.synthetic.ready","predicate":"at_least","expected":3}),
            EvaluatorRefusal("at_least requires a numeric_values target".into()),
        ),
        (
            json!({"target":"set_memberships.synthetic.tools","predicate":"member_of","expected":{}}),
            EvaluatorRefusal("member_of expected value must be a JSON scalar".into()),
        ),
    ] {
        assert_eq!(
            advisor::validate_tree(&raw, &state, InstanceMode::UserMode),
            Err(reason.clone())
        );
        assert!(!advisor::refusal_diagnostic(Some(A), &reason)
            .to_string()
            .contains("synthetic-private-expected"));
    }
    let affect =
        json!({"target":"facts.affect.synthetic_ready","predicate":"equals","expected":true});
    assert_eq!(
        advisor::validate_tree(&affect, &state, InstanceMode::WorkerMode),
        Err(ModeRefusal)
    );
    assert!(advisor::validate_tree(&affect, &state, InstanceMode::UserMode).is_ok());
}

#[tokio::test]
async fn rechecked_ineligible_tasks_are_bounded_to_three_names_and_one_count() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 30.0).await;
    for n in 1..10 {
        seed(&state, &id(n), "active", json!({})).await;
    }
    let (context, _) = advisor::select(&state, 25).await.unwrap();
    let sub = advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let mut result = stub.submit(&sub).unwrap();
    for n in 0..10 {
        sqlx::query("UPDATE objects SET status='completed', payload_json=json_set(payload_json, '$.status', 'completed') WHERE id=?").bind(id(n)).execute(state.inner().store.pool()).await.unwrap();
    }
    advisor::vet_result(&state, &mut result).await.unwrap();
    assert!(result.proposed_candidates.is_empty());
    assert_eq!(result.diagnostics.len(), 4);
    assert_eq!(
        result.diagnostics[3],
        json!({"code":"precondition_task_skipped","message":"7 more Tasks are no longer eligible: they are inactive, absent, or routine occurrences. Nothing was changed."})
    );
}
