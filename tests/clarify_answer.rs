//! P1B-48 §C: answering is admitting. Synthetic and offline.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use ubu_orchestrator::services::{
    advisory_wire::{Question, QuestionKind},
    clarify::MAX_DESCRIPTION_BYTES,
    proposal_applier::compose,
};

/// One Task with one question set waiting, and the candidate's id.
async fn interview(sets: Vec<Value>) -> (ubu_orchestrator::state::AppState, std::sync::Arc<Stub>, String) {
    let stub = Stub::answering(sets);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({"tags":["synthetic-tag"]})).await;
    let run = clarify(&state, None).await;
    let id = run["candidate_ids"][0].as_str().unwrap().to_owned();
    (state, stub, id)
}
fn code(body: &Value) -> &str {
    body["diagnostics"][0]["code"].as_str().unwrap_or("<none>")
}
async fn untouched(state: &ubu_orchestrator::state::AppState, id: &str, before: &Value, ledger_before: i64) {
    assert_eq!(candidates(state).await, vec![(id.to_owned(), "proposed".to_owned(), 1)]);
    assert_eq!(&task(state, A).await, before);
    assert_eq!(ledger(state).await, ledger_before);
}

#[tokio::test]
async fn plain_admit_refuses_a_clarification_and_changes_nothing() {
    let (state, _, id) = interview(vec![questions()]).await;
    let before = task(&state, A).await;
    let ledger_before = ledger(&state).await;
    let (status, body) = request(&state, "POST", &format!("/advisory/candidate/{id}/admit"), json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["diagnostics"],
        json!([{"code":"advisory_answer_required","message":"A clarification proposal is admitted by answering its questions, not by Admit"}])
    );
    untouched(&state, &id, &before, ledger_before).await;
}

#[tokio::test]
async fn answering_a_tag_proposal_refuses_and_changes_nothing() {
    // A tag candidate, from the tag producer.
    let tags = std::sync::Arc::new(TagStub);
    let state = bare().await.with_advisory_transport_factory(std::sync::Arc::new(move |_| tags.clone()));
    configure(&state).await;
    seed(&state, A, "active", json!({})).await;
    let (status, run) = run_with(&state, json!({"producer":"suggest_tags"})).await;
    assert_eq!(status, StatusCode::OK);
    let id = run["candidate_ids"][0].as_str().unwrap().to_owned();
    let before = task(&state, A).await;
    let ledger_before = ledger(&state).await;
    let (status, body) = answer(&state, &id, 1, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "advisory_not_a_clarification");
    untouched(&state, &id, &before, ledger_before).await;
}
struct TagStub;
impl ubu_core::worker::AdvisoryTransport for TagStub {
    fn submit(&self, sub: &ubu_core::worker::LocalAdvisorySubmission) -> ubu_core::Result<ubu_core::worker::LocalAdvisoryResult> {
        let proposals: Vec<_> = sub.payload.as_array().unwrap().iter().map(|t| json!({"id":t["id"],"category_tag":"work","confidence":0.8})).collect();
        let body = json!({"done":true,"response":json!({"proposals":proposals}).to_string()});
        Ok(ubu_orchestrator::services::advisory_wire::interpret(sub, 200, &serde_json::to_vec(&body).unwrap()))
    }
}

#[tokio::test]
async fn each_refused_answer_set_has_its_own_diagnostic_and_leaves_the_candidate_proposed() {
    let (state, _, id) = interview(vec![questions()]).await;
    let before = task(&state, A).await;
    let ledger_before = ledger(&state).await;
    let long = "x".repeat(MAX_DESCRIPTION_BYTES);
    for (answers, expected) in [
        (json!({"q1":"maybe"}), "clarify_invalid_answer"),
        (json!({"q1":"yes"}), "clarify_invalid_answer"),
        // Invalid even where the question would not have applied: a radio group cannot send it.
        (json!({"q1":"n","q3":"Synthetic recipient","q9":"Synthetic answer to nothing"}), "clarify_unknown_question"),
        (json!({}), "clarify_no_answers"),
        (json!({"q1":"  ","q2":"","q3":"\n"}), "clarify_no_answers"),
        // Only a question that does not apply is answered: its dependency was left blank.
        (json!({"q2":"Synthetic Friday"}), "clarify_no_answers"),
        (json!({"q1":"n","q2":"Synthetic Friday","q3":long}), "clarify_description_too_large"),
    ] {
        let (status, body) = answer(&state, &id, 1, answers.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answers}: {body}");
        assert_eq!(body["diagnostics"].as_array().unwrap().len(), 1, "{answers}");
        assert_eq!(code(&body), expected, "{answers}");
        untouched(&state, &id, &before, ledger_before).await;
        println!("P1B48_C_REFUSED {expected}: {}", body["diagnostics"][0]["message"].as_str().unwrap().chars().take(200).collect::<String>());
    }
    let (_, body) = answer(&state, &id, 1, json!({})).await;
    assert!(body["diagnostics"][0]["message"].as_str().unwrap().contains("Defer"));
    assert!(body["diagnostics"][0]["message"].as_str().unwrap().contains("Reject"));
    // A body that is not the request is refused before anything is read.
    for malformed in [json!({"observed_version":1}), json!({"observed_version":1,"answers":{"q1":true}}), json!({"observed_version":1,"answers":{},"note":"synthetic"})] {
        let (status, _) = request(&state, "POST", &format!("/advisory/candidate/{id}/answer"), malformed).await;
        assert!(status.is_client_error());
    }
    untouched(&state, &id, &before, ledger_before).await;
}

#[tokio::test]
async fn a_good_answer_set_admits_and_writes_the_interview_in_question_order() {
    let (state, _, id) = interview(vec![questions()]).await;
    // Given out of order, in mixed case and padded; q3 blank; q4 does not apply because q1 is y.
    let (status, body) = answer(&state, &id, 1, json!({"q4":"Synthetic reason that never applied","q3":"  ","q2":"  next synthetic Friday ","q1":" Y "})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let expected = "Q: Is there a deadline for the synthetic teapot?\nA: y\nQ: What is the synthetic deadline?\nA: next synthetic Friday\n";
    assert_eq!(body["task"]["description"], expected);
    assert_eq!(body["candidate"]["lifecycle_state"], "admitted");
    assert_eq!(body["state_category"], "candidate_state");
    assert_eq!(candidates(&state).await, vec![(id.clone(), "admitted".to_owned(), 2)]);
    let stored = task(&state, A).await;
    assert_eq!(stored["description"], expected);
    assert_eq!(stored["__version"], 2);
    // Nothing else about the Task moved, and no tag was smuggled in with the interview.
    assert_eq!(stored["tags"], json!(["synthetic-tag"]));
    assert_eq!(stored["title"], "Synthetic lunar teapot 0");
    assert!(stored.get("category_tag").is_none());
    println!("P1B48_C_DESCRIPTION={}", json!(expected));
    // Admitted is final: it cannot be answered, admitted, or answered at its new version.
    for version in [1, 2] {
        let (status, again) = answer(&state, &id, version, json!({"q1":"n"})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{version}: {again}");
    }
    assert_eq!(task(&state, A).await["description"], expected);
}

#[tokio::test]
async fn a_second_round_appends_and_does_not_replace() {
    let (state, stub, first) = interview(vec![questions(), second_round()]).await;
    let (status, _) = answer(&state, &first, 1, json!({"q1":"n","q3":"Synthetic recipient","q4":"Synthetic reason"})).await;
    assert_eq!(status, StatusCode::OK);
    let round_one = "Q: Is there a deadline for the synthetic teapot?\nA: n\nQ: Who is the synthetic teapot for?\nA: Synthetic recipient\nQ: Why is there no synthetic deadline?\nA: Synthetic reason\n";
    assert_eq!(task(&state, A).await["description"], round_one);
    let run = clarify(&state, Some(A)).await;
    let second = run["candidate_ids"][0].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    // Round two was asked with round one's answers in hand.
    assert_eq!(stub.prompt(1), json!({"id":A,"title":"Synthetic lunar teapot 0","tags":["synthetic-tag"],"description":round_one,"round":2}));
    let (status, body) = answer(&state, &second, 1, json!({"r1":"y"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let both = format!("{round_one}Q: Is the synthetic teapot already bought?\nA: y\n");
    assert_eq!(task(&state, A).await["description"], both);
    assert_eq!(task(&state, A).await["__version"], 3);
    assert_eq!(ubu_orchestrator::services::clarify::rounds_admitted(&state, A).await.unwrap(), 2);
}

#[tokio::test]
async fn what_the_operator_typed_is_kept_and_a_missing_line_end_is_supplied() {
    let stub = Stub::answering([second_round()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({"description":"Synthetic note with no line end"})).await;
    let id = clarify(&state, Some(A)).await["candidate_ids"][0].as_str().unwrap().to_owned();
    let (status, _) = answer(&state, &id, 1, json!({"r1":"N"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        task(&state, A).await["description"],
        "Synthetic note with no line end\nQ: Is the synthetic teapot already bought?\nA: n\n"
    );
}

#[tokio::test]
async fn a_stale_observed_version_is_a_conflict() {
    let (state, _, id) = interview(vec![questions()]).await;
    let (status, _) = request(&state, "POST", &format!("/advisory/candidate/{id}/defer"), json!({"observed_version":1})).await;
    assert_eq!(status, StatusCode::OK);
    let before = task(&state, A).await;
    let (status, body) = answer(&state, &id, 1, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "PreconditionFailed");
    assert_eq!(task(&state, A).await, before);
    assert_eq!(candidates(&state).await, vec![(id.clone(), "deferred".to_owned(), 2)]);
    // Deferred is "not now": it is resurfaced before it is answered.
    let (status, body) = answer(&state, &id, 2, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, _) = request(&state, "POST", &format!("/advisory/candidate/{id}/resurface"), json!({"observed_version":2,"trigger":"user_request"})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = answer(&state, &id, 3, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // An id that names no candidate, and one that is no id.
    let (status, _) = answer(&state, "advcand_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f", 1, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = answer(&state, "synthetic", 1, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_task_that_is_no_longer_active_is_not_written_to() {
    let (state, _, id) = interview(vec![questions()]).await;
    let (status, _) = request(&state, "POST", &format!("/task/{A}/action"), json!({"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"})).await;
    assert_eq!(status, StatusCode::OK);
    let before = task(&state, A).await;
    let (status, body) = answer(&state, &id, 1, json!({"q1":"y"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "advisory_target_inactive");
    assert_eq!(task(&state, A).await, before);
    assert_eq!(candidates(&state).await, vec![(id, "proposed".to_owned(), 1)]);
}

#[test]
fn compose_is_pure_and_its_rules_hold_one_by_one() {
    let question = |id: &str, text: &str, kind, depends_on: Option<(&str, &str)>| Question {
        id: id.into(),
        text: text.into(),
        kind,
        depends_on: depends_on.map(|(a, b)| (a.to_owned(), b.to_owned())),
    };
    // Ids deliberately out of alphabetical order: the narrative follows the questions.
    let set = vec![
        question("z", "Synthetic first?", QuestionKind::YesNo, None),
        question("m", "Synthetic second?", QuestionKind::ShortText, Some(("z", "Y"))),
        question("a", "Synthetic third?", QuestionKind::ShortText, Some(("m", "Blue"))),
        question("b", "Synthetic fourth?", QuestionKind::ShortText, Some(("z", "n"))),
    ];
    let given = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    };
    assert_eq!(
        compose("", &set, &given(&[("a", "deep"), ("m", "BLUE"), ("z", "y")])).unwrap(),
        "Q: Synthetic first?\nA: y\nQ: Synthetic second?\nA: BLUE\nQ: Synthetic third?\nA: deep\n"
    );
    // A chain is cut where it breaks: m does not apply, so a, which depends on m, does not either.
    assert_eq!(
        compose("", &set, &given(&[("a", "deep"), ("m", "blue"), ("z", "n"), ("b", "because")])).unwrap(),
        "Q: Synthetic first?\nA: n\nQ: Synthetic fourth?\nA: because\n"
    );
    // A blank dependency satisfies nothing.
    assert!(compose("", &set, &given(&[("z", " "), ("m", "blue")])).is_err());
    // Existing text that ends a line is not given a second line end.
    assert_eq!(
        compose("Synthetic earlier\n", &set, &given(&[("z", "N")])).unwrap(),
        "Synthetic earlier\nQ: Synthetic first?\nA: n\n"
    );
    // Exactly at the limit is accepted; one byte over is not.
    let room = MAX_DESCRIPTION_BYTES - "Q: Synthetic first?\nA: y\n".len();
    assert_eq!(compose(&"x".repeat(room - 1), &set, &given(&[("z", "y")])).unwrap().len(), MAX_DESCRIPTION_BYTES);
    assert!(compose(&"x".repeat(room), &set, &given(&[("z", "y")])).is_err());
}
