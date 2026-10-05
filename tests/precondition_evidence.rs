#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};

#[tokio::test]
async fn title_only_candidate_records_only_the_title_it_was_given() {
    let (state, _) = ready(tree()).await;
    fact(&state, 30.0).await;
    seed(&state, B, "active", json!({})).await;
    assert_eq!(run(&state).await["candidates_enqueued"], 2);
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    let candidate = queue["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row["candidate"])
        .find(|candidate| candidate["target_refs"][0]["id"] == B)
        .unwrap();
    assert_eq!(candidate["evidence_refs"], json!([format!("{B}:title")]));
}

#[tokio::test]
async fn described_candidate_records_both_title_and_description() {
    let (state, _) = ready(tree()).await;
    fact(&state, 30.0).await;
    assert_eq!(run(&state).await["candidates_enqueued"], 1);
    let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
    assert_eq!(
        queue["candidates"][0]["candidate"]["evidence_refs"],
        json!([format!("{A}:title"), format!("{A}:description")])
    );
}
