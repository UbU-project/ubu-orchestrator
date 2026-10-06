//! Synthetic teapot data; StubTransport is the only transport.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{
    services::{advisory_wire, precondition_advisor},
    state::AppState,
};
struct StubTransport {
    bad: bool,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        let proposals = if self.bad {
            if sub.expected_result_schema == precondition_advisor::RESULT_SCHEMA {
                json!([{"id":A,"precondition":{"target":TARGET,"predicate":"equals"}}])
            } else {
                json!([{"id":A,"target":"facts.affect.synthetic"}])
            }
        } else {
            json!([])
        };
        Ok(advisory_wire::interpret(
            sub,
            200,
            &serde_json::to_vec(
                &json!({"done":true,"response":json!({"proposals":proposals}).to_string()}),
            )
            .unwrap(),
        ))
    }
}
async fn prepared(bad: bool) -> AppState {
    let (state, _) = ready(tree()).await;
    fact(&state, 7.0).await;
    for n in 0..8 {
        seed(
            &state,
            &format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d{:04x}", 0x6e80 + n),
            "active",
            json!({"title":"   "}),
        )
        .await;
    }
    let stub = Arc::new(StubTransport { bad });
    state.with_advisory_transport_factory(Arc::new(move |_| stub.clone()))
}
async fn run_producer(state: &AppState, producer: &str) -> Value {
    let (status, body) = request(
        state,
        "POST",
        "/advisory/run",
        json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":producer,"limit":25}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}
#[tokio::test]
async fn each_producer_reports_one_shared_gate_decision_with_three_names_then_count() {
    for producer in ["vocabulary", "precondition"] {
        let state = prepared(false).await;
        let before = canonical_rows(&state).await;
        let body = run_producer(&state, producer).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["candidates_enqueued"], 0);
        let notes = body["diagnostics"].as_array().unwrap();
        assert_eq!(notes.len(), 4, "{body}");
        for (n, note) in notes.iter().take(3).enumerate() {
            assert_eq!(note["code"], "advisory_task_skipped");
            assert_eq!(note["message"],format!("Task `task_018f3c8e9b2a7c4d8f1e2a3b4c5d{:04x}` has neither a title nor a description to reason over",0x6e80+n));
        }
        assert_eq!(
            notes[3],
            json!({"code":"advisory_task_skipped","message":"5 more Tasks were skipped: they are routine occurrences or have neither a title nor a description"})
        );
        assert!(!notes[3]["message"].as_str().unwrap().contains("task_"));
        assert_eq!(canonical_rows(&state).await, before);
    }
}
#[tokio::test]
async fn shared_gate_does_not_rename_producer_specific_proposal_refusals() {
    for (producer, code) in [
        ("vocabulary", "vocabulary_proposal_refused"),
        ("precondition", "precondition_proposal_refused"),
    ] {
        let state = prepared(true).await;
        let body = run_producer(&state, producer).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(
            body["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["code"] == "advisory_task_skipped")
                .count(),
            4
        );
        assert!(diagnostic(&body, code), "{body}");
    }
}
#[tokio::test]
async fn independent_runs_report_fresh_identical_gate_notes_without_hidden_cross_run_state() {
    let state = prepared(false).await;
    let first = run_producer(&state, "vocabulary").await;
    let second = run_producer(&state, "precondition").await;
    assert_eq!(first["diagnostics"], second["diagnostics"]);
    assert_eq!(second["diagnostics"].as_array().unwrap().len(), 4);
}
