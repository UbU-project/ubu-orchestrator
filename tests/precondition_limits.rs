//! Invented queue fixtures only; the existing StubTransport is the only transport.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use ubu_orchestrator::{
    config::ServerConfig,
    services::{advisory_wire, precondition_advisor as advisor, setting_authoring},
    state::AppState,
};

async fn seed_queue(state: &AppState, lifecycles: &[&str]) {
    let context = advisor::Context {
        tasks: vec![advisor::DescribedTask {
            id: A.into(),
            title: "Synthetic queued teapot".into(),
            description: None,
        }],
        targets: vec![TARGET.into()],
    };
    let sub = advisor::submission(state, &context, "synthetic-model")
        .await
        .unwrap();
    for (index, lifecycle) in lifecycles.iter().enumerate() {
        let mut requirement = tree();
        requirement["expected"] = json!(index + 1);
        let bytes = serde_json::to_vec(&json!({"done":true,"response":json!({"proposals":[{"id":A,"precondition":requirement}]}).to_string()})).unwrap();
        let candidate = advisory_wire::interpret(&sub, 200, &bytes)
            .proposed_candidates
            .remove(0);
        let id = format!("advcand_018f3c8e9b2a7c4d8f1e2a3b{:08x}", index);
        let mut payload = serde_json::to_value(&candidate).unwrap();
        payload["advisory_candidate_id"] = json!(id);
        payload["lifecycle_state"] = json!(lifecycle);
        payload["idempotency_key"] = json!(format!("synthetic-queue-{index}"));
        sqlx::query("INSERT INTO advisory_candidates (advisory_candidate_id,candidate_kind,lifecycle_state,version,suppression_key,review_order,payload_json,created_at,updated_at) VALUES (?,'precondition',?,1,?,?,?, ?,?)")
            .bind(id).bind(lifecycle).bind(&candidate.suppression_key).bind(index as i64)
            .bind(payload.to_string()).bind(NOW).bind(NOW)
            .execute(state.inner().store.pool()).await.unwrap();
    }
}

#[tokio::test]
async fn twenty_five_considered_tasks_have_a_three_proposal_schema_and_prompt_bound() {
    let (state, _) = ready(tree()).await;
    let context = advisor::Context {
        tasks: (0..25)
            .map(|index| advisor::DescribedTask {
                id: format!("task_018f3c8e9b2a7c4d8f1e2a3b{:08x}", index),
                title: format!("Synthetic selected teapot {index}"),
                description: None,
            })
            .collect(),
        targets: vec![TARGET.into()],
    };
    let sub = advisor::submission(&state, &context, "synthetic-model")
        .await
        .unwrap();
    let body = advisory_wire::request_body(&sub).unwrap();
    assert_eq!(body["format"]["properties"]["proposals"]["maxItems"], 3);
    let prompt: Value = serde_json::from_str(body["prompt"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["tasks"].as_array().unwrap().len(), 25);
    assert!(body["system"].as_str().unwrap().contains("The response is bounded to at most three proposals in total, regardless of how many Tasks are supplied."));
}

#[tokio::test]
async fn four_model_proposals_refuse_the_whole_response_without_writes() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 30.0).await;
    for index in 1..4 {
        seed(
            &state,
            &format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d{:04x}", 0x6e70 + index),
            "active",
            json!({}),
        )
        .await;
    }
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let result = run(&state).await;
    assert_eq!(result["selected"].as_array().unwrap().len(), 4);
    assert_eq!(result["status"], "malformed_result");
    assert!(diagnostic(&result, "advisory_malformed_result"));
    assert_eq!(result["candidates_enqueued"], 0);
    assert_eq!(count(&state, "advisory_candidates").await, 0);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
    assert_eq!(canonical_rows(&state).await, before);
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn ten_or_more_awaiting_including_resurfaced_refuses_before_transport_construction() {
    for total in [10, 12] {
        let (state, stub) = ready(tree()).await;
        seed_queue(
            &state,
            &(0..total)
                .map(|i| if i % 2 == 0 { "proposed" } else { "resurfaced" })
                .collect::<Vec<_>>(),
        )
        .await;
        let called = Arc::new(AtomicUsize::new(0));
        let calls = called.clone();
        let transport = stub.clone();
        let state = state.with_advisory_transport_factory(Arc::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            transport.clone()
        }));
        let before = canonical_rows(&state).await;
        let ledger = count(&state, "mutation_envelopes").await;
        let result = run(&state).await;
        assert_eq!(result["status"], "ok");
        assert_eq!(result["selected"], json!([]));
        assert_eq!(result["candidates_enqueued"], 0);
        assert_eq!(result["report"], Value::Null);
        assert_eq!(
            result["diagnostics"],
            json!([{"code":"precondition_queue_full","message":format!("{total} precondition candidates are waiting in Review; review, defer or reject them before asking for more. No model was asked.")}])
        );
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert!(stub.submissions.lock().unwrap().is_empty());
        assert_eq!(count(&state, "advisory_candidates").await, total as i64);
        assert_eq!(count(&state, "mutation_envelopes").await, ledger);
        assert_eq!(canonical_rows(&state).await, before);
    }
}

#[tokio::test]
async fn a_full_queue_refuses_before_consulting_an_absent_transport_factory() {
    let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
    for (name, value) in [
        ("advisory.model", "synthetic-model"),
        ("advisory.endpoint", "http://127.0.0.1:11434"),
    ] {
        setting_authoring::put(&state, name, json!(value))
            .await
            .unwrap();
    }
    seed_queue(&state, &["proposed"; 10]).await;
    let result = run(&state).await;
    assert!(diagnostic(&result, "precondition_queue_full"));
    assert!(!diagnostic(&result, "advisory_transport_unavailable"));
}

#[tokio::test]
async fn nine_awaiting_permits_a_model_call_and_enqueue() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 30.0).await;
    seed_queue(&state, &["proposed"; 9]).await;
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 1, "{result}");
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
    assert_eq!(advisor::awaiting_review(&state).await.unwrap(), 10);
}

#[tokio::test]
async fn ten_deferred_candidates_do_not_block_a_model_call() {
    let (state, stub) = ready(tree()).await;
    fact(&state, 30.0).await;
    seed_queue(&state, &["deferred"; 10]).await;
    assert_eq!(advisor::awaiting_review(&state).await.unwrap(), 0);
    let result = run(&state).await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["candidates_enqueued"], 1, "{result}");
    assert_eq!(stub.submissions.lock().unwrap().len(), 1);
}
