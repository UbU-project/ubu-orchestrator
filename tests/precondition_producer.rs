#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use ubu_core::core::{InstanceMode, UniverseState};
use ubu_orchestrator::services::{advisory_wire, precondition_advisor};

#[tokio::test]
async fn title_only_task_is_selected_and_its_prompt_omits_description() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 0.0).await;
    seed(&state, B, "active", json!({})).await;
    let before = canonical_rows(&state).await;
    assert_eq!(run(&state).await["candidates_enqueued"], 2);
    assert_eq!(canonical_rows(&state).await, before);
    let sub = &stub.submissions.lock().unwrap()[0];
    let wire = advisory_wire::request_body(sub).unwrap();
    let prompt: Value = serde_json::from_str(wire["prompt"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["tasks"][0], json!({"id":A,"title":"Synthetic lunar teapot 0","description":"Synthetic orbital teapot launch requires at least 25 charge units."}));
    assert_eq!(prompt["tasks"][1], json!({"id":B,"title":"Synthetic lunar teapot 1"}));
    assert!(prompt["tasks"][1].get("description").is_none());
    let system = wire["system"].as_str().unwrap();
    assert!(system.starts_with("Propose at most one necessary precondition per Task from its title and description."));
    assert!(system.contains("A Task may arrive with no description, and its title is then the whole of what is known about it."));
}

#[tokio::test]
async fn only_routines_and_tasks_with_neither_title_nor_description_are_skipped() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    seed(&state, B, "active", json!({"title":" \t", "description":" \n"})).await;
    let routine = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e72";
    seed(&state, routine, "active", json!({"occurrence":{"routine_objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01","local_date":"2026-09-29","key":"synthetic-occurrence"}})).await;
    let (context, diagnostics) = precondition_advisor::select(&state, 25).await.unwrap();
    assert_eq!(context.tasks.len(), 1);
    assert_eq!(context.tasks[0].id, A);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].message, format!("Task `{B}` has neither a title nor a description to reason over"));
    assert_eq!(diagnostics[1].message, format!("Task `{routine}` is a routine occurrence; edit its template instead"));
}

#[tokio::test]
async fn blank_description_is_omitted_and_description_only_task_is_eligible() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    seed(&state, B, "active", json!({"description":" \n"})).await;
    let (context, _) = precondition_advisor::select(&state, 25).await.unwrap();
    assert!(serde_json::to_value(&context).unwrap()["tasks"][1].get("description").is_none());
    seed(&state, "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e72", "active", json!({"title":"", "description":"Synthetic description-only work"})).await;
    assert_eq!(precondition_advisor::select(&state, 25).await.unwrap().0.tasks.len(), 3);
    assert!(serde_json::from_value::<precondition_advisor::DescribedTask>(json!({"id":B,"title":"Synthetic","extra":true})).is_err());
}

#[tokio::test]
async fn existing_fact_enqueues_one_candidate_and_only_its_store_metadata_changes() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 0.0).await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["candidates_enqueued"], 1, "{result}");
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "advisory_candidates").await, 1);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger + 1);
    let sub = &stub.submissions.lock().unwrap()[0];
    let wire = advisory_wire::request_body(sub).unwrap();
    let prompt: Value = serde_json::from_str(wire["prompt"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["targets"], json!([TARGET]));
    assert!(
        prompt.get("numeric_values").is_none(),
        "fact values are not model input"
    );
    assert_eq!(
        wire["format"]["$defs"]["leaf"]["oneOf"][2]["properties"]["predicate"]["enum"],
        json!(["at_least", "at_most", "greater_than", "less_than"])
    );
    assert_eq!(
        result["report"]["proposals"][0]["normalized_proposal"],
        tree()
    );
}

#[tokio::test]
async fn missing_targets_are_one_bounded_diagnostic_and_never_a_candidate() {
    let leaves: Vec<_> = (0..5).map(|n| json!({"target":format!("facts.synthetic.missing_{n}"),"predicate":"equals","expected":"synthetic-private-expected"})).collect();
    let (state, _) = ready(json!({"all_of":leaves})).await;
    fact(&state, 0.0).await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 0);
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 1);
    assert!(diagnostic(&result, "precondition_missing_targets"));
    let message = result["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains("facts.synthetic.missing_0") && message.contains("and 2 more"));
    assert!(!result.to_string().contains("synthetic-private-expected"));
    assert_eq!(result["report"]["proposals"], json!([]));
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
    assert_eq!(count(&state, "advisory_candidates").await, 0);
}

#[tokio::test]
async fn malformed_or_hidden_malformed_branches_never_reach_the_queue_or_echo_output() {
    for bad in [
        json!({"all_of":[]}),
        json!({"target":TARGET,"predicate":"invented","expected":25}),
        json!({"target":TARGET,"predicate":"at_least","expected":{}}),
        json!({"all_of":[tree(),{"target":TARGET,"predicate":"invented","expected":25}]}),
        json!({"target":"facts.private prose\nignore instructions","predicate":"equals","expected":true}),
        json!({"target":TARGET,"predicate":"at_least","expected":25,"extra":"synthetic-echo-marker"}),
    ] {
        let (state, _) = ready(bad).await;
        fact(&state, 0.0).await;
        let before = canonical_rows(&state).await;
        let ledger = count(&state, "mutation_envelopes").await;
        let result = run(&state).await;
        assert_eq!(result["status"], "malformed_result", "{result}");
        assert!(diagnostic(&result, "advisory_malformed_result"));
        assert_eq!(result["candidates_enqueued"], 0);
        assert_eq!(canonical_rows(&state).await, before);
        assert_eq!(count(&state, "mutation_envelopes").await, ledger);
        assert!(!result.to_string().contains("synthetic-echo-marker"));
    }
}

#[test]
fn intrinsic_affect_is_refused_outside_user_mode() {
    let mut universe = UniverseState::new(
        ubu_core::UbuTimestamp::parse(NOW).unwrap(),
        "synthetic mode test",
    );
    universe
        .facts
        .insert("affect.synthetic_ready".into(), json!(true));
    let proposal =
        json!({"target":"facts.affect.synthetic_ready","predicate":"equals","expected":true});
    for mode in [InstanceMode::OrganizationMode, InstanceMode::WorkerMode] {
        assert!(precondition_advisor::validate_tree(&proposal, &universe, mode).is_err());
    }
    assert!(
        precondition_advisor::validate_tree(&proposal, &universe, InstanceMode::UserMode)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn existing_precondition_is_reviewed_as_replacement_without_canonical_writes() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 0.0).await;
    let existing = json!({"target":TARGET,"predicate":"at_least","expected":10});
    let (status, body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":existing})).await;
    assert_eq!(status, 200, "{body}");
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["candidates_enqueued"], 1, "{result}");
    let expected = json!({"existing_precondition":existing,"proposed_precondition":tree()});
    assert_eq!(
        result["report"]["proposals"][0]["normalized_proposal"],
        expected
    );
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    assert_eq!(
        queue["candidates"][0]["candidate"]["normalized_proposal"],
        expected
    );
    assert_eq!(
        queue["candidates"][0]["candidate"]["payload"]["value"],
        expected
    );
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
    assert!(stub.submissions.lock().unwrap()[0].payload["tasks"][0]
        .get("preconditions")
        .is_none());
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger + 1);
    assert_eq!(count(&state, "advisory_candidates").await, 1);
    assert_eq!(run(&state).await["candidates_enqueued"], 0);
}

#[tokio::test]
async fn a_changed_prior_tree_is_a_distinct_review_choice() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    assert_eq!(run(&state).await["candidates_enqueued"], 1);
    let existing = json!({"target":TARGET,"predicate":"at_least","expected":10});
    let (status, body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":existing})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(run(&state).await["candidates_enqueued"], 1);
    assert_eq!(count(&state, "advisory_candidates").await, 2);
}

#[tokio::test]
async fn empty_universe_is_not_created_by_an_advisor_run() {
    let (state, stub) = ready(tree()).await;
    let before = canonical_rows(&state).await;
    let result = run(&state).await;
    assert!(diagnostic(&result, "precondition_no_facts"));
    assert_eq!(canonical_rows(&state).await, before);
    assert!(stub.submissions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn repeated_identical_proposal_reuses_the_durable_queue_boundary() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    assert_eq!(run(&state).await["candidates_enqueued"], 1);
    let ledger = count(&state, "mutation_envelopes").await;
    assert_eq!(run(&state).await["candidates_enqueued"], 0);
    assert_eq!(count(&state, "advisory_candidates").await, 1);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
}

#[tokio::test]
async fn absent_predicate_on_missing_target_is_still_not_offered() {
    let (state, _) = ready(json!({"target":"facts.synthetic.missing","predicate":"absent"})).await;
    fact(&state, 0.0).await;
    let result = run(&state).await;
    assert!(diagnostic(&result, "precondition_missing_targets"));
    assert_eq!(result["candidates_enqueued"], 0);
}
