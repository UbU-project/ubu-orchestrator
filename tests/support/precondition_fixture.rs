//! Invented orbital-teapot data only. No network client is constructible here.
#![allow(dead_code)]
#[path = "clarify_fixture.rs"]
mod base;
#[allow(unused_imports)]
pub use base::{request, seed, task, A, B, NOW};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::{Arc, Mutex};
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::{services::advisory_wire, state::AppState};

pub const TARGET: &str = "numeric_values.synthetic.orbital_teapot_charge";
pub fn tree() -> Value {
    json!({"target":TARGET,"predicate":"at_least","expected":25})
}
pub struct StubTransport {
    pub tree: Value,
    pub submissions: Mutex<Vec<LocalAdvisorySubmission>>,
}
impl AdvisoryTransport for StubTransport {
    fn submit(
        &self,
        submission: &LocalAdvisorySubmission,
    ) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(submission.clone());
        let proposals: Vec<_> = submission.payload["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| json!({"id":task["id"],"precondition":self.tree}))
            .collect();
        let bytes = serde_json::to_vec(
            &json!({"done":true,"response":json!({"proposals":proposals}).to_string()}),
        )
        .unwrap();
        Ok(advisory_wire::interpret(submission, 200, &bytes))
    }
}
pub async fn ready(proposal: Value) -> (AppState, Arc<StubTransport>) {
    let stub = Arc::new(StubTransport {
        tree: proposal,
        submissions: Mutex::new(vec![]),
    });
    let transport = stub.clone();
    let state = base::bare()
        .await
        .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
    base::configure(&state).await;
    seed(&state, A, "active", json!({"description":"Synthetic orbital teapot launch requires at least 25 charge units.","duration_estimate":{"type":"fixed","seconds":600}})).await;
    (state, stub)
}
pub async fn fact(state: &AppState, value: f64) {
    let (status, body) = request(state,"PATCH","/universe-state",json!({"schema_version":"ubu.orchestrator.universe_state.v1","mutations":[{"operation":"set_numeric","target":TARGET,"payload":value}]})).await;
    assert_eq!(status, 200, "{body}");
}
pub async fn run(state: &AppState) -> Value {
    let (status, body) = request(state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"precondition","limit":25})).await;
    assert_eq!(status, 200, "{body}");
    body
}
pub async fn count(state: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
/// Snapshot every non-candidate table, field for field. The only excluded table
/// beside candidates is the mutation ledger, whose exact enqueue delta is checked separately.
pub async fn canonical_rows(state: &AppState) -> Vec<(String, Vec<String>)> {
    let pool = state.inner().store.pool();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT IN ('advisory_candidates','mutation_envelopes','sqlite_sequence') ORDER BY name").fetch_all(pool).await.unwrap();
    let mut snapshot = vec![];
    for name in names {
        let columns = sqlx::query(&format!("PRAGMA table_info(\"{name}\")"))
            .fetch_all(pool)
            .await
            .unwrap();
        let expressions = columns
            .iter()
            .map(|row| format!("quote(\"{}\")", row.get::<String, _>("name")))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let mut rows: Vec<String> =
            sqlx::query_scalar(&format!("SELECT {expressions} FROM \"{name}\""))
                .fetch_all(pool)
                .await
                .unwrap();
        rows.sort();
        snapshot.push((name, rows));
    }
    snapshot
}
pub fn diagnostic(body: &Value, code: &str) -> bool {
    body["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == code)
}
pub async fn plan(state: &AppState) -> Value {
    let (status, body) = request(
        state,
        "POST",
        "/planning/generate",
        json!({"schema_version":"planning-kernel-contract/0.1","request":null}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}
pub async fn action(
    state: &AppState,
    id: &str,
    action: &str,
    version: u64,
) -> (axum::http::StatusCode, Value) {
    let mut body = json!({"observed_version":version});
    if action == "reject" {
        body["reason"] = json!("Synthetic review refusal");
        body["retention_policy"] = json!("retain");
    }
    if action == "resurface" {
        body["trigger"] = json!("user_request");
    }
    request(
        state,
        "POST",
        &format!("/advisory/candidate/{id}/{action}"),
        body,
    )
    .await
}
