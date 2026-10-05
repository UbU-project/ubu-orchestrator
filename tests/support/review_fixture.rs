#![allow(dead_code)]
#[path = "clarify_fixture.rs"]
mod base;
#[path = "precondition_fixture.rs"]
mod precondition;
#[allow(unused_imports)]
pub use precondition::{
    action, canonical_rows, count, diagnostic, fact, plan, request, seed, task, tree, A, B, NOW,
    TARGET,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{services::advisory_wire, state::AppState};
pub struct StubTransport {
    pub answer: Mutex<Value>,
    pub submissions: Mutex<Vec<LocalAdvisorySubmission>>,
}
impl AdvisoryTransport for StubTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(sub.clone());
        let answer = self.answer.lock().unwrap().clone();
        let reviews = sub.payload["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let mut r = answer.clone();
                r["id"] = t["id"].clone();
                r
            })
            .collect::<Vec<_>>();
        Ok(advisory_wire::interpret(
            sub,
            200,
            &serde_json::to_vec(
                &json!({"done":true,"response":json!({"reviews":reviews}).to_string()}),
            )
            .unwrap(),
        ))
    }
}
pub fn removal() -> Value {
    json!({"verdict":"remove","reason":"This synthetic requirement is unrelated to the stated work."})
}
pub async fn ready(answer: Value) -> (AppState, Arc<StubTransport>) {
    let stub = Arc::new(StubTransport {
        answer: Mutex::new(answer),
        submissions: Mutex::new(vec![]),
    });
    let transport = stub.clone();
    let state = base::bare()
        .await
        .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
    base::configure(&state).await;
    seed(&state,A,"active",json!({"description":"Synthetic orbital teapot inspection needs no charge.","preconditions":tree(),"duration_estimate":{"type":"fixed","seconds":600}})).await;
    fact(&state, 0.0).await;
    (state, stub)
}
pub async fn run(state: &AppState) -> Value {
    let (status,body)=request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"precondition_review","limit":25})).await;
    assert_eq!(status, 200, "{body}");
    body
}
