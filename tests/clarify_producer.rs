//! P1B-48 §B: the clarify producer, through POST /advisory/run. Synthetic and offline.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use ubu_core::worker::AdvisoryCapability;
use ubu_core::CandidateKind;
use ubu_orchestrator::services::{advisory_service::run_advisory, clarify};

fn codes(response: &Value) -> Vec<&str> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn automatic_selection_is_the_first_task_with_no_description_and_never_an_occurrence() {
    let stub = Stub::answering([questions()]);
    let state = ready(stub.clone()).await;
    // A sorts first and is an occurrence; B is described; C is the first that qualifies.
    seed(&state, A, "active", occurrence()).await;
    seed(&state, B, "active", json!({"description":"Q: Synthetic question?\nA: y\n"})).await;
    seed(&state, C, "active", json!({"category_tag":"grocery","tags":["grocery","synthetic-tag"]})).await;
    let response = clarify(&state, None).await;
    assert_eq!(response["status"], "ok");
    assert_eq!(response["selected"], json!([{"id":C,"title":"Synthetic lunar teapot 2"}]));
    assert_eq!(response["candidates_enqueued"], 1);
    assert_eq!(response["diagnostics"], json!([]));
    assert_eq!(stub.asked(), 1);
    // What left the process: the one Task, its category and tags, the round, and no description.
    assert_eq!(
        stub.prompt(0),
        json!({"id":C,"title":"Synthetic lunar teapot 2","category_tag":"grocery","tags":["grocery","synthetic-tag"],"round":1})
    );
    println!("P1B48_B_PROMPT_ROUND_1={}", stub.prompt(0));
    assert_eq!(candidates(&state).await.len(), 1);
    // A blank description counts as none.
    let state = ready(Stub::answering([questions()])).await;
    seed(&state, A, "active", json!({"description":"  \n"})).await;
    assert_eq!(clarify(&state, None).await["selected"][0]["id"], A);
}

#[tokio::test]
async fn with_nothing_to_select_the_run_says_which_kind_of_nothing_and_asks_no_model() {
    let stub = Stub::answering([]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", occurrence()).await;
    seed(&state, B, "active", json!({"description":"Q: Synthetic question?\nA: y\n"})).await;
    seed(&state, C, "completed", json!({})).await;
    let none = clarify(&state, None).await;
    assert_eq!(none["status"], "ok");
    assert_eq!(
        none["diagnostics"],
        json!([{"code":"clarify_no_task","message":"Every active Task already has a description; name a Task to interview it again"}])
    );
    // Named, an occurrence and a completed Task are each refused by name. An absent Task too.
    for id in [A, C, "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f"] {
        let named = clarify(&state, Some(id)).await;
        assert_eq!(
            named["diagnostics"],
            json!([{"code":"clarify_no_task","message":format!("Task `{id}` is not an active, non-routine Task")}])
        );
        assert_eq!(named["selected"], json!([]));
        assert_eq!(named["candidates_enqueued"], 0);
    }
    // A malformed id, and an id of another kind, are request errors.
    for id in ["synthetic", "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01", ""] {
        let (status, body) = run_with(&state, json!({"producer":"clarify","task_id":id})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}: {body}");
        assert_eq!(body["diagnostics"][0]["code"], "clarify_invalid_task_id", "{id}");
    }
    assert_eq!(stub.asked(), 0);
    assert!(candidates(&state).await.is_empty());
}

#[tokio::test]
async fn naming_a_task_interviews_it_whatever_its_description() {
    let stub = Stub::answering([questions()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    seed(&state, B, "active", json!({"description":"Synthetic notes the operator typed\n"})).await;
    let response = clarify(&state, Some(B)).await;
    assert_eq!(response["selected"], json!([{"id":B,"title":"Synthetic lunar teapot 1"}]));
    assert_eq!(response["candidates_enqueued"], 1);
    // The description is sent, and the round is 1: no interview has been answered yet.
    assert_eq!(
        stub.prompt(0),
        json!({"id":B,"title":"Synthetic lunar teapot 1","description":"Synthetic notes the operator typed\n","round":1})
    );
}

#[tokio::test]
async fn the_round_is_one_and_then_two_after_one_admission() {
    let stub = Stub::answering([questions(), second_round()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    seed(&state, B, "active", json!({})).await;
    assert_eq!(clarify::rounds_admitted(&state, A).await.unwrap(), 0);
    let first = clarify(&state, Some(A)).await;
    assert_eq!(first["report"]["proposals"][0]["normalized_proposal"]["round"], 1);
    assert_eq!(stub.prompt(0)["round"], 1);
    // Proposed is not admitted: the round does not advance until the operator answers.
    assert_eq!(clarify::rounds_admitted(&state, A).await.unwrap(), 0);
    // Answering is section C. Here the candidate is marked admitted directly, which is
    // all the count reads.
    sqlx::query("UPDATE advisory_candidates SET lifecycle_state='admitted', version=2 WHERE advisory_candidate_id=?")
        .bind(first["candidate_ids"][0].as_str().unwrap())
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(clarify::rounds_admitted(&state, A).await.unwrap(), 1);
    // Another Task's interview is not this Task's round.
    assert_eq!(clarify::rounds_admitted(&state, B).await.unwrap(), 0);
    let second = clarify(&state, Some(A)).await;
    assert_eq!(second["report"]["proposals"][0]["normalized_proposal"]["round"], 2);
    assert_eq!(stub.prompt(1)["round"], 2);
    assert_eq!(stub.asked(), 2);
}

#[tokio::test]
async fn an_open_interview_refuses_another_run_without_asking_the_model() {
    let stub = Stub::answering([questions(), second_round()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    let first = clarify(&state, None).await;
    let id = first["candidate_ids"][0].as_str().unwrap().to_owned();
    assert_eq!(stub.asked(), 1);
    let expected = json!([{"code":"clarify_already_queued","message":format!("Task `{A}` (Synthetic lunar teapot 0) already has questions waiting in Review; answer, defer or reject them before asking for more")}]);
    // Proposed, then deferred, then resurfaced: each is an open interview.
    let transitions: [(&str, Value); 3] = [
        ("", Value::Null),
        ("defer", json!({"observed_version":1})),
        ("resurface", json!({"observed_version":2,"trigger":"user_request"})),
    ];
    for (action, body) in transitions {
        if !action.is_empty() {
            let (status, moved) = request(&state, "POST", &format!("/advisory/candidate/{id}/{action}"), body).await;
            assert_eq!(status, StatusCode::OK, "{action}: {moved}");
        }
        for named in [None, Some(A)] {
            let refused = clarify(&state, named).await;
            assert_eq!(refused["status"], "ok", "{action}");
            assert_eq!(refused["diagnostics"], expected, "{action}");
            assert_eq!(refused["candidates_enqueued"], 0);
            assert_eq!(refused["selected"], json!([]));
        }
    }
    assert_eq!(stub.asked(), 1);
    assert_eq!(candidates(&state).await.len(), 1);
    // Rejecting it closes the interview, and the next run asks again.
    let (status, _) = request(&state, "POST", &format!("/advisory/candidate/{id}/reject"), json!({"observed_version":3,"reason":"Synthetic: not these questions","retention_policy":"retain"})).await;
    assert_eq!(status, StatusCode::OK);
    let again = clarify(&state, None).await;
    assert_eq!(again["candidates_enqueued"], 1);
    assert_eq!(stub.asked(), 2);
}

#[tokio::test]
async fn a_rejected_question_set_does_not_return_and_is_not_mistaken_for_done() {
    let stub = Stub::answering([questions(), questions()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    let id = clarify(&state, None).await["candidate_ids"][0].as_str().unwrap().to_owned();
    request(&state, "POST", &format!("/advisory/candidate/{id}/reject"), json!({"observed_version":1,"reason":"Synthetic: not these questions","retention_policy":"retain"})).await;
    // The model asks the very same questions again; the durable rejection holds.
    let again = clarify(&state, None).await;
    assert_eq!(again["candidates_enqueued"], 0);
    assert_eq!(codes(&again), ["advisory_proposal_suppressed"]);
    assert_eq!(candidates(&state).await.len(), 1);
}

#[tokio::test]
async fn done_enqueues_nothing_and_is_not_a_failure() {
    let stub = Stub::answering([json!({"done":true,"questions":[]})]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    let before = task(&state, A).await;
    let response = clarify(&state, None).await;
    assert_eq!(response["status"], "ok");
    assert_eq!(response["candidates_enqueued"], 0);
    assert_eq!(response["selected"], json!([{"id":A,"title":"Synthetic lunar teapot 0"}]));
    assert_eq!(
        response["diagnostics"],
        json!([{"code":"clarify_no_questions","message":format!("The model has no further question about Task `{A}`; nothing was enqueued and the Task is unchanged")}])
    );
    assert_eq!(stub.asked(), 1);
    assert!(candidates(&state).await.is_empty());
    assert_eq!(task(&state, A).await, before);
    // A malformed answer is a failure, and is not dressed as "no questions".
    let state = ready(Stub::answering([json!({"questions":"synthetic"})])).await;
    seed(&state, A, "active", json!({})).await;
    let failed = clarify(&state, None).await;
    assert_eq!(failed["status"], "malformed_result");
    assert_eq!(codes(&failed), ["advisory_malformed_result"]);
}

#[tokio::test]
async fn a_field_that_belongs_to_the_other_producer_is_refused() {
    let stub = Stub::answering([]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    for (fields, code) in [
        (json!({"producer":"clarify","limit":1}), "advisory_limit_unsupported"),
        (json!({"producer":"clarify","limit":1,"task_id":A}), "advisory_limit_unsupported"),
        (json!({"producer":"suggest_tags","task_id":A}), "advisory_task_id_unsupported"),
        (json!({"producer":"advise"}), "advisory_unknown_producer"),
    ] {
        let (status, body) = run_with(&state, fields.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{fields}: {body}");
        assert_eq!(body["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(body["diagnostics"][0]["code"], code, "{fields}");
    }
    let (_, unknown) = run_with(&state, json!({"producer":"advise"})).await;
    assert_eq!(unknown["diagnostics"][0]["message"], "The producers are suggest_tags and clarify");
    let (status, _) = run_with(&state, json!({"producer":"clarify","round":2})).await;
    assert!(status.is_client_error(), "an unknown field is refused: {status}");
    assert_eq!(stub.asked(), 0);
    assert!(candidates(&state).await.is_empty());
}

#[tokio::test]
async fn the_guards_before_the_model_are_the_same_for_both_producers() {
    // Nothing is selected and nothing is asked, whichever producer: unconfigured, an
    // endpoint that is not loopback, and a process with no transport.
    let mut outcomes = Vec::new();
    for producer in ["suggest_tags", "clarify"] {
        let mut seen = Vec::new();
        let stub = Stub::answering([]);

        let state = inject(bare().await, stub.clone());
        seed(&state, A, "active", json!({})).await;
        let (status, unconfigured) = run_with(&state, json!({"producer":producer})).await;
        seen.push((status, unconfigured["status"].clone(), codes(&unconfigured).join(","), unconfigured["selected"].clone()));
        let names: Vec<_> = unconfigured["diagnostics"].as_array().unwrap().iter().map(|d| d["message"].as_str().unwrap().split(' ').next().unwrap().to_owned()).collect();
        assert_eq!(names, ["advisory.model", "advisory.endpoint"]);

        configure(&state).await;
        sqlx::query("UPDATE objects SET payload_json=json_set(payload_json,'$.value','http://synthetic.invalid:11434') WHERE object_type='Setting' AND json_extract(payload_json,'$.name')='advisory.endpoint'")
            .execute(state.inner().store.pool()).await.unwrap();
        let (status, invalid) = run_with(&state, json!({"producer":producer})).await;
        seen.push((status, invalid["status"].clone(), codes(&invalid).join(","), invalid["selected"].clone()));

        let bare_state = bare().await;
        configure(&bare_state).await;
        seed(&bare_state, A, "active", json!({})).await;
        let (status, unavailable) = run_with(&bare_state, json!({"producer":producer})).await;
        seen.push((status, unavailable["status"].clone(), codes(&unavailable).join(","), unavailable["selected"].clone()));

        assert_eq!(stub.asked(), 0, "{producer}");
        assert!(candidates(&state).await.is_empty());
        outcomes.push(seen);
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(
        outcomes[1].iter().map(|outcome| outcome.2.as_str()).collect::<Vec<_>>(),
        ["advisory_unconfigured,advisory_unconfigured", "advisory_endpoint_invalid", "advisory_transport_unavailable"]
    );
    assert!(outcomes[1].iter().all(|outcome| outcome.0 == StatusCode::OK && outcome.3 == json!([])));
}

#[tokio::test]
async fn a_question_set_proposed_under_a_tag_only_authority_is_rejected_and_never_stored() {
    let stub = Stub::answering([questions()]);
    let state = ready(stub.clone()).await;
    seed(&state, A, "active", json!({})).await;
    let context = clarify::select(&state, None).await.unwrap().unwrap();
    let mut submission = clarify::submission(&state, &context, "synthetic-model:1").await.unwrap();
    assert!(submission.authority.may_propose(CandidateKind::ClarificationQuestion));
    assert!(!submission.authority.may_propose(CandidateKind::Tag));
    assert_eq!(submission.timeout_ms, 120_000);
    assert_eq!(submission.compute_budget.max_cpu_ms, 120_000);
    // The worker is granted tags only, and answers with questions all the same.
    submission.authority.granted = [
        AdvisoryCapability::ProposeCandidate(CandidateKind::Tag),
        AdvisoryCapability::EmitDiagnostics,
    ]
    .into_iter()
    .collect();
    let report = run_advisory(&state, submission, stub.as_ref()).await.unwrap();
    assert_eq!(report.candidates_stored, 0);
    assert_eq!(report.candidates_rejected, 1);
    assert!(report.candidate_ids.is_empty());
    assert!(candidates(&state).await.is_empty());
    println!("P1B48_B_AUTHORITY stored={} rejected={} status={:?} validation_error={:?}", report.candidates_stored, report.candidates_rejected, report.status, report.validation_error);
}
