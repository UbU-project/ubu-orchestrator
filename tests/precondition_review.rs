#[path = "support/review_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use ubu_orchestrator::services::{advisory_wire, precondition_review};

#[tokio::test]
async fn review_enqueue_is_the_only_store_change_and_prompt_matches_visible_words() {
    let (state,stub)=ready(json!({"verdict":"replace","reason":"The synthetic requirement needs a lower threshold.","proposed_precondition":json!({"target":TARGET,"predicate":"at_least","expected":0})})).await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["candidates_enqueued"], 1, "{result}");
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger + 1);
    assert_eq!(count(&state, "advisory_candidates").await, 1);
    let p = &result["report"]["proposals"][0]["normalized_proposal"];
    assert_eq!(p["existing_precondition"], tree());
    assert_eq!(p["verdict"], "replace");
    assert_eq!(p["proposed_precondition"]["expected"], 0);
    let wire = advisory_wire::request_body(&stub.submissions.lock().unwrap()[0]).unwrap();
    let prompt: Value = serde_json::from_str(wire["prompt"].as_str().unwrap()).unwrap();
    assert_eq!(
        prompt["tasks"][0]["existing_precondition"],
        format!("{TARGET} is at least 25")
    );
    assert_eq!(prompt["targets"], json!([TARGET]));
    assert!(prompt.get("numeric_values").is_none());
    assert!(!result["diagnostics"]
        .to_string()
        .contains("lower threshold"));
}
#[tokio::test]
async fn removal_has_reason_and_existing_tree_but_no_proposed_tree() {
    let (state, _) = ready(removal()).await;
    let r = run(&state).await;
    assert_eq!(r["candidates_enqueued"], 1, "{r}");
    let p = &r["report"]["proposals"][0]["normalized_proposal"];
    assert_eq!(p["operation"], "clear_precondition");
    assert_eq!(p["existing_precondition"], tree());
    assert!(p.get("proposed_precondition").is_none());
    assert_eq!(p["reason"], removal()["reason"]);
    assert_eq!(p["blocked_now"], true);
}
#[tokio::test]
async fn sound_is_an_aggregate_diagnostic_without_candidate_or_model_prose() {
    let (state, _) =
        ready(json!({"verdict":"sound","reason":"Synthetic private explanation."})).await;
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let r = run(&state).await;
    assert_eq!(r["candidates_enqueued"], 0);
    assert!(diagnostic(&r, "precondition_review_sound"));
    assert!(!r.to_string().contains("private explanation"));
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
}
#[tokio::test]
async fn review_refuses_absent_targets_including_absent_predicate() {
    for predicate in ["equals", "absent"] {
        let mut tree = json!({"target":"facts.synthetic.missing","predicate":predicate});
        if predicate == "equals" {
            tree["expected"] = json!(true);
        }
        let (state, _) = ready(
            json!({"verdict":"replace","reason":"Synthetic change.","proposed_precondition":tree}),
        )
        .await;
        let r = run(&state).await;
        assert_eq!(r["candidates_enqueued"], 0);
        assert!(diagnostic(&r, "precondition_missing_targets"));
    }
}
#[tokio::test]
async fn review_refuses_malformed_trees_and_missing_or_blank_reasons() {
    for answer in [
        json!({"verdict":"replace","reason":"Synthetic change.","proposed_precondition":{"all_of":[]}}),
        json!({"verdict":"remove"}),
        json!({"verdict":"remove","reason":"  "}),
        json!({"verdict":"remove","reason":"Synthetic change.","confidence":0.8}),
        json!({"verdict":"unknown","reason":"Synthetic change."}),
    ] {
        let (state, _) = ready(answer).await;
        let r = run(&state).await;
        assert_eq!(r["candidates_enqueued"], 0);
        assert!(diagnostic(&r, "advisory_malformed_result"), "{r}");
    }
}
#[tokio::test]
async fn task_without_precondition_is_never_reviewed() {
    let (state, stub) = ready(removal()).await;
    seed(
        &state,
        B,
        "active",
        json!({"description":"Synthetic unrelated task."}),
    )
    .await;
    run(&state).await;
    assert_eq!(
        stub.submissions.lock().unwrap()[0].payload["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn words_match_the_screen_for_all_predicates_and_boolean_groups() {
    assert_eq!(precondition_review::words(&json!({"all_of":[{"target":"facts.synthetic.ready","predicate":"equals","expected":true},{"any_of":[{"target":TARGET,"predicate":"less_than","expected":5},{"target":"facts.synthetic.absent","predicate":"absent"}]}]})),format!("facts.synthetic.ready is true and {TARGET} is less than 5 or facts.synthetic.absent is not set"));
    for (p, w) in [
        ("at_least", "is at least"),
        ("at_most", "is at most"),
        ("greater_than", "is greater than"),
        ("less_than", "is less than"),
    ] {
        assert_eq!(
            precondition_review::words(&json!({"target":TARGET,"predicate":p,"expected":5})),
            format!("{TARGET} {w} 5")
        );
    }
}
#[test]
fn canonical_review_fixtures_roundtrip_through_existing_core_candidate() {
    for raw in [
        include_str!(
            "fixtures/review-replace.json"
        ),
        include_str!("fixtures/review-remove.json"),
    ] {
        let c: ubu_core::AdvisoryCandidate = serde_json::from_str(raw).unwrap();
        c.validate().unwrap();
        let again: ubu_core::AdvisoryCandidate =
            serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(c, again);
    }
}
