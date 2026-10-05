#[path = "support/review_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};

fn placed(plan: &Value) -> bool {
    plan["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["task_id"] == A)
}
#[tokio::test]
async fn admitting_removal_restores_excluded_task_to_the_next_plan() {
    let (state, _) = ready(removal()).await;
    seed(&state, B, "active", json!({"duration_estimate":{"type":"fixed","seconds":300}})).await;
    assert!(!placed(&plan(&state).await));
    let (_, universe) = request(&state, "GET", "/universe-state", Value::Null).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    let (status, admitted) = action(&state, id, "admit", 1).await;
    assert_eq!(status, 200, "{admitted}");
    assert!(admitted["task"]["preconditions"].is_null());
    assert!(placed(&plan(&state).await));
    assert_eq!(
        request(&state, "GET", "/universe-state", Value::Null)
            .await
            .1,
        universe
    );
    assert_eq!(action(&state, id, "admit", 2).await.0, 409);
}
#[tokio::test]
async fn admitting_review_replacement_changes_next_plan_eligibility() {
    let proposed = json!({"target":TARGET,"predicate":"at_most","expected":25});
    let (state,_)=ready(json!({"verdict":"replace","reason":"The synthetic threshold is reversed.","proposed_precondition":proposed})).await;
    seed(&state, B, "active", json!({"duration_estimate":{"type":"fixed","seconds":300}})).await;
    assert!(!placed(&plan(&state).await));
    let r = run(&state).await;
    let (status, body) = action(&state, r["candidate_ids"][0].as_str().unwrap(), "admit", 1).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["task"]["preconditions"], proposed);
    assert!(placed(&plan(&state).await));
}
#[tokio::test]
async fn rejecting_review_preserves_task_and_operator_reason() {
    let (state, _) = ready(removal()).await;
    let r = run(&state).await;
    let before = task(&state, A).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    assert_eq!(action(&state, id, "reject", 1).await.0, 200);
    assert_eq!(task(&state, A).await, before);
    let raw: String = sqlx::query_scalar("SELECT payload_json FROM suppression_records")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap()["rejection_reason_or_user_correction"],
        "Synthetic review refusal"
    );
}
#[tokio::test]
async fn deferred_review_requires_resurface_before_removal() {
    let (state, _) = ready(removal()).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    assert_eq!(action(&state, id, "defer", 1).await.0, 200);
    let before = task(&state, A).await;
    assert_eq!(action(&state, id, "admit", 2).await.0, 409);
    assert_eq!(task(&state, A).await, before);
    assert_eq!(action(&state, id, "resurface", 2).await.0, 200);
    assert_eq!(action(&state, id, "admit", 3).await.0, 200);
}
#[tokio::test]
async fn changed_reviewed_value_refuses_removal_and_cleared_fact_does_not() {
    let (state, _) = ready(removal()).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    let (status,body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":{"target":TARGET,"predicate":"at_least","expected":26}})).await;
    assert_eq!(status, 200, "{body}");
    let before = task(&state, A).await;
    assert_eq!(action(&state, id, "admit", 1).await.0, 409);
    assert_eq!(task(&state, A).await, before);
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    request(&state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"clear_numeric","target":TARGET}]})).await;
    let (status, body) = action(&state, id, "admit", 1).await;
    assert_eq!(status, 200, "{body}");
}
