#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};

#[tokio::test]
async fn admit_then_plan_excludes_when_false_and_includes_when_true_over_http() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    seed(
        &state,
        B,
        "active",
        json!({"duration_estimate":{"type":"fixed","seconds":300}}),
    )
    .await;
    let proposal = run(&state).await;
    let id = proposal["candidate_ids"][0].as_str().unwrap();
    let task_before = task(&state, A).await;
    let (_, universe_before) = request(&state, "GET", "/universe-state", Value::Null).await;
    let (status, admitted) = action(&state, id, "admit", 1).await;
    assert_eq!(status, 200, "{admitted}");
    assert_eq!(admitted["task"]["preconditions"], tree());
    let mut task_after = task(&state, A).await;
    task_after.as_object_mut().unwrap().remove("preconditions");
    // The ordinary writer updates envelope metadata; placement and descriptive
    // fields, including all declarations, remain the same.
    for key in [
        "title",
        "description",
        "static_window",
        "duration_estimate",
        "status",
        "tags",
        "effects",
    ] {
        assert_eq!(task_after[key], task_before[key], "{key}");
    }
    assert_eq!(
        request(&state, "GET", "/universe-state", Value::Null)
            .await
            .1,
        universe_before
    );
    let blocked = plan(&state).await;
    assert!(
        blocked["blocked_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["task_id"] == A),
        "{blocked}"
    );
    assert!(
        !blocked["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["task_id"] == A),
        "{blocked}"
    );
    fact(&state, 25.0).await;
    let ready = plan(&state).await;
    assert!(
        ready["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["task_id"] == A),
        "{ready}"
    );
    assert!(ready.get("blocked_tasks").is_none_or(|tasks| tasks
        .as_array()
        .unwrap()
        .iter()
        .all(|task| task["task_id"] != A)));
    println!(
        "P1B61_ADMISSION: false excludes; true includes; UniverseState unchanged by admission"
    );
}

#[tokio::test]
async fn reject_keeps_task_untouched_and_suppresses_identical_proposal() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    let proposal = run(&state).await;
    let before = task(&state, A).await;
    let (status, _) = action(
        &state,
        proposal["candidate_ids"][0].as_str().unwrap(),
        "reject",
        1,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(task(&state, A).await, before);
    assert_eq!(run(&state).await["candidates_enqueued"], 0);
}

#[tokio::test]
async fn deferred_requires_resurface_and_admission_twice_is_refused() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    let proposal = run(&state).await;
    let id = proposal["candidate_ids"][0].as_str().unwrap();
    assert_eq!(action(&state, id, "defer", 1).await.0, 200);
    let before = task(&state, A).await;
    let (status, error) = action(&state, id, "admit", 2).await;
    assert_eq!(status, 409, "{error}");
    assert_eq!(task(&state, A).await, before);
    assert_eq!(action(&state, id, "resurface", 2).await.0, 200);
    let (status, admitted) = action(&state, id, "admit", 3).await;
    assert_eq!(status, 200, "{admitted}");
    let before = task(&state, A).await;
    assert_eq!(action(&state, id, "admit", 4).await.0, 409);
    assert_eq!(task(&state, A).await, before);
}

#[tokio::test]
async fn fact_cleared_after_proposal_refuses_admission_without_writes() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    let proposal = run(&state).await;
    let (status, body)=request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"clear_numeric","target":TARGET}]})).await;
    assert_eq!(status, 200, "{body}");
    let before = canonical_rows(&state).await;
    let (status, body) = action(
        &state,
        proposal["candidate_ids"][0].as_str().unwrap(),
        "admit",
        1,
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert!(diagnostic(&body, "advisory_precondition_stale"));
    assert_eq!(canonical_rows(&state).await, before);
}

#[tokio::test]
async fn replacement_admission_writes_only_the_reviewed_new_tree() {
    let (state, _) = ready(tree()).await;
    fact(&state, 0.0).await;
    let existing = json!({"target":TARGET,"predicate":"at_least","expected":10});
    let (status, body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":existing})).await;
    assert_eq!(status, 200, "{body}");
    let before = task(&state, A).await;
    let universe = request(&state, "GET", "/universe-state", Value::Null)
        .await
        .1;
    let proposal = run(&state).await;
    assert_eq!(
        proposal["report"]["proposals"][0]["normalized_proposal"]["existing_precondition"],
        existing
    );
    assert_eq!(task(&state, A).await, before);
    let (status, admitted) = action(
        &state,
        proposal["candidate_ids"][0].as_str().unwrap(),
        "admit",
        1,
    )
    .await;
    assert_eq!(status, 200, "{admitted}");
    assert_eq!(task(&state, A).await["preconditions"], tree());
    for key in [
        "title",
        "description",
        "static_window",
        "duration_estimate",
        "status",
        "tags",
        "effects",
    ] {
        assert_eq!(task(&state, A).await[key], before[key], "{key}");
    }
    assert_eq!(
        request(&state, "GET", "/universe-state", Value::Null)
            .await
            .1,
        universe
    );
}

#[tokio::test]
async fn a_condition_added_changed_or_cleared_after_review_refuses_admission_without_writes() {
    for (already_present, clear) in [(false, false), (true, false), (true, true)] {
        let (state, _) = ready(tree()).await;
        fact(&state, 0.0).await;
        let prior = json!({"target":TARGET,"predicate":"at_least","expected":10});
        if already_present {
            let (status, body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":prior})).await;
            assert_eq!(status, 200, "{body}");
        }
        let proposal = run(&state).await;
        let later = if clear {
            Value::Null
        } else {
            json!({"target":TARGET,"predicate":"at_least","expected":50})
        };
        let (status, body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":if already_present {2} else {1},"preconditions":later})).await;
        assert_eq!(status, 200, "{body}");
        let before = canonical_rows(&state).await;
        let ledger = count(&state, "mutation_envelopes").await;
        let (_, queue_before) = request(&state, "GET", "/advisory/queue", Value::Null).await;
        let (status, body) = action(
            &state,
            proposal["candidate_ids"][0].as_str().unwrap(),
            "admit",
            1,
        )
        .await;
        assert_eq!(status, 409, "{body}");
        assert!(diagnostic(&body, "advisory_precondition_changed"));
        assert_eq!(canonical_rows(&state).await, before);
        assert_eq!(count(&state, "mutation_envelopes").await, ledger);
        assert_eq!(
            request(&state, "GET", "/advisory/queue", Value::Null)
                .await
                .1,
            queue_before
        );
    }
}
