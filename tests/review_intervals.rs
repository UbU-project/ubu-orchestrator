#[path = "support/review_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use ubu_core::UbuTimestamp;
use ubu_orchestrator::{planning_time::FixedClock, services::setting_authoring, state::AppState};

async fn policy(state: &AppState, id: &str) -> Value {
    request(state, "GET", "/advisory/queue", Value::Null)
        .await
        .1["review_intervals"][id]
        .clone()
}
async fn force(state: &AppState) -> Value {
    let (status,r)=request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"precondition_review","force":true})).await;
    assert_eq!(status, 200, "{r}");
    r
}
async fn dismiss(
    state: &AppState,
    id: &str,
    verb: &str,
    version: u64,
    days: Option<u64>,
    reason: &str,
) -> Value {
    let mut body = json!({"observed_version":version});
    if let Some(days) = days {
        body["snooze_days"] = json!(days);
    }
    if verb == "reject" {
        body["reason"] = json!(reason);
        body["retention_policy"] = json!("retain");
    }
    let (status, r) = request(
        state,
        "POST",
        &format!("/advisory/candidate/{id}/{verb}"),
        body,
    )
    .await;
    assert_eq!(status, 200, "{r}");
    r
}
fn later(state: &AppState, days: i64) -> AppState {
    let now = chrono::DateTime::parse_from_rfc3339(NOW).unwrap() + chrono::Duration::days(days);
    state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(now.to_rfc3339()).unwrap()))
}
#[tokio::test]
async fn dismissed_review_is_held_despite_different_model_wording() {
    let (state, stub) = ready(removal()).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    dismiss(
        &state,
        id,
        "reject",
        1,
        None,
        "The synthetic condition is intentional.",
    )
    .await;
    stub.answer.lock().unwrap()["reason"] = json!("A differently worded synthetic critique.");
    let held = run(&state).await;
    assert_eq!(held["candidates_enqueued"], 0);
    assert!(diagnostic(&held, "advisory_proposal_suppressed"));
    assert!(held["diagnostics"].to_string().contains("2026-10-06"));
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
    let r = force(&state).await;
    assert_eq!(r["candidates_enqueued"], 1, "{r}");
    let subs = stub.submissions.lock().unwrap();
    let wire = ubu_orchestrator::services::advisory_wire::request_body(&subs[1]).unwrap();
    assert!(wire["prompt"]
        .as_str()
        .unwrap()
        .contains("The synthetic condition is intentional."));
}
#[tokio::test]
async fn same_dismissal_lapses_at_saved_interval_and_renewed_rejection_succeeds() {
    let (state, _) = ready(removal()).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    dismiss(&state, id, "reject", 1, Some(3), "").await;
    setting_authoring::put(&state, setting_authoring::REVIEW_SEED, json!(14))
        .await
        .unwrap();
    assert_eq!(run(&later(&state, 2)).await["candidates_enqueued"], 0);
    let r = run(&later(&state, 3)).await;
    assert_eq!(r["candidates_enqueued"], 1, "{r}");
    dismiss(
        &later(&state, 3),
        r["candidate_ids"][0].as_str().unwrap(),
        "reject",
        1,
        None,
        "Another correction.",
    )
    .await;
    assert_eq!(count(&state, "suppression_records").await, 1);
    assert_eq!(count(&state, "candidate_decision_events").await, 2);
}
#[tokio::test]
async fn changed_value_invalidates_hold_while_escalation_survives() {
    let (state, _) = ready(removal()).await;
    fact(&state, 100.0).await;
    let r = run(&state).await;
    let old = r["candidate_ids"][0].as_str().unwrap();
    dismiss(&state, old, "reject", 1, None, "").await;
    let (status,body)=request(&state,"PATCH",&format!("/task/{A}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"preconditions":{"target":TARGET,"predicate":"at_least","expected":26}})).await;
    assert_eq!(status, 200, "{body}");
    let r = run(&state).await;
    assert_eq!(r["candidates_enqueued"], 1, "{r}");
    let id = r["candidate_ids"][0].as_str().unwrap();
    assert_eq!(policy(&state, id).await["suggested_days"], 14);
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT suppression_key FROM advisory_candidates ORDER BY rowid")
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    assert_ne!(keys[0], keys[1]);
    assert!(keys
        .iter()
        .all(|k| k.starts_with("review:sha256:") && !k.contains(TARGET)));
}
#[tokio::test]
async fn dismissal_series_doubles_to_ceiling_counting_repeated_defer_events() {
    let (state, _) = ready(removal()).await;
    fact(&state, 100.0).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    let mut observed = Vec::new();
    let mut version = 1;
    for expected in [7, 14, 28, 56, 112, 224, 365, 365] {
        let p = policy(&state, id).await;
        observed.push(p["suggested_days"].as_u64().unwrap());
        assert_eq!(p["suggested_days"], expected);
        dismiss(&state, id, "defer", version, None, "").await;
        version += 1;
        force(&state).await;
        version += 1;
    }
    assert_eq!(observed, vec![7, 14, 28, 56, 112, 224, 365, 365]);
    assert_eq!(count(&state, "advisory_candidates").await, 1);
    println!("P1B62_INTERVAL_SERIES={observed:?}");
}
#[tokio::test]
async fn admitted_review_resets_escalation_to_seed() {
    let (state,_)=ready(json!({"verdict":"replace","reason":"A synthetic correction.","proposed_precondition":{"target":TARGET,"predicate":"at_least","expected":20}})).await;
    fact(&state, 100.0).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    dismiss(&state, id, "defer", 1, None, "").await;
    force(&state).await;
    assert_eq!(policy(&state, id).await["suggested_days"], 14);
    assert_eq!(action(&state, id, "admit", 3).await.0, 200);
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    assert_eq!(policy(&state, id).await["suggested_days"], 7);
}
#[tokio::test]
async fn blocking_now_cap_overrides_escalation_and_stale_span_choice() {
    let (state, _) = ready(removal()).await;
    fact(&state, 100.0).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    dismiss(&state, id, "defer", 1, None, "").await;
    force(&state).await;
    assert_eq!(policy(&state, id).await["suggested_days"], 14);
    fact(&state, 0.0).await;
    let p = policy(&state, id).await;
    assert_eq!(p["suggested_days"], 7);
    assert_eq!(p["capped"], true);
    dismiss(&state, id, "defer", 3, Some(14), "").await;
    assert_eq!(
        policy(&state, id).await["held_until"],
        "2026-10-06T08:00:00Z"
    );
}
#[tokio::test]
async fn expired_deferral_resurfaces_with_policy_trigger_and_user_request_bypasses_hold() {
    let (state, stub) = ready(removal()).await;
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    dismiss(&state, id, "defer", 1, None, "").await;
    assert_eq!(run(&state).await["candidates_enqueued"], 0);
    let r = run(&later(&state, 7)).await;
    assert!(diagnostic(&r, "precondition_review_resurfaced"));
    let c = request(&state, "GET", "/advisory/queue", Value::Null)
        .await
        .1["candidates"][0]["candidate"]
        .clone();
    assert_eq!(c["links"]["resurface_trigger"], "policy_review_interval");
    assert_eq!(c["version"], 3);
    dismiss(&state, id, "defer", 3, None, "").await;
    force(&state).await;
    let c = request(&state, "GET", "/advisory/queue", Value::Null)
        .await
        .1["candidates"][0]["candidate"]
        .clone();
    assert_eq!(c["links"]["resurface_trigger"], "user_request");
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn settings_validate_days_pair_defaults_and_custom_curve() {
    let (state, _) = ready(removal()).await;
    fact(&state, 100.0).await;
    for invalid in [json!(0), json!(366), json!(2.5), json!("3"), json!(true)] {
        assert!(
            setting_authoring::put(&state, setting_authoring::REVIEW_SEED, invalid)
                .await
                .is_err()
        );
    }
    setting_authoring::put(&state, setting_authoring::REVIEW_SEED, json!(3))
        .await
        .unwrap();
    setting_authoring::put(&state, setting_authoring::REVIEW_CEILING, json!(5))
        .await
        .unwrap();
    assert!(
        setting_authoring::put(&state, setting_authoring::REVIEW_SEED, json!(6))
            .await
            .is_err()
    );
    assert!(
        setting_authoring::delete(&state, setting_authoring::REVIEW_SEED)
            .await
            .is_err()
    );
    let r = run(&state).await;
    let id = r["candidate_ids"][0].as_str().unwrap();
    assert_eq!(policy(&state, id).await["suggested_days"], 3);
    dismiss(&state, id, "defer", 1, None, "").await;
    force(&state).await;
    assert_eq!(policy(&state, id).await["suggested_days"], 5);
}
