//! Shared by the P1B-48 route tests. Synthetic and offline: the only transport is this stub.
#![allow(dead_code)]
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{advisory_wire as wire, setting_authoring},
    state::AppState,
};

pub const NOW: &str = "2026-09-29T08:00:00Z";
pub const A: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
pub const B: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e71";
pub const C: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e72";
pub const ENDPOINT: &str = "http://127.0.0.1:11434";

/// Answers every submission with the next queued body, handed to the wire layer.
#[derive(Default)]
pub struct Stub {
    pub submissions: Mutex<Vec<LocalAdvisorySubmission>>,
    pub answers: Mutex<Vec<Value>>,
}
impl Stub {
    pub fn answering(sets: impl IntoIterator<Item = Value>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(sets.into_iter().collect()),
            ..Default::default()
        })
    }
    pub fn asked(&self) -> usize {
        self.submissions.lock().unwrap().len()
    }
    /// The prompt of the nth request, as the model would have received it.
    pub fn prompt(&self, index: usize) -> Value {
        let body = wire::request_body(&self.submissions.lock().unwrap()[index]).unwrap();
        serde_json::from_str(body["prompt"].as_str().unwrap()).unwrap()
    }
}
impl AdvisoryTransport for Stub {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        self.submissions.lock().unwrap().push(sub.clone());
        let mut answers = self.answers.lock().unwrap();
        assert!(!answers.is_empty(), "the stub was asked more often than the test expected");
        let set = answers.remove(0);
        let body = json!({"done":true,"response":set.to_string()});
        Ok(wire::interpret(sub, 200, &serde_json::to_vec(&body).unwrap()))
    }
}

pub fn questions() -> Value {
    json!({"done":false,"questions":[
        {"id":"q1","text":"Is there a deadline for the synthetic teapot?","kind":"YesNo"},
        {"id":"q2","text":"What is the synthetic deadline?","kind":"ShortText","depends_on":["q1","y"]},
        {"id":"q3","text":"Who is the synthetic teapot for?","kind":"ShortText"},
        {"id":"q4","text":"Why is there no synthetic deadline?","kind":"ShortText","depends_on":["q1","n"]}
    ]})
}
pub fn second_round() -> Value {
    json!({"done":false,"questions":[
        {"id":"r1","text":"Is the synthetic teapot already bought?","kind":"YesNo"}
    ]})
}

pub async fn bare() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
pub fn inject(state: AppState, stub: Arc<Stub>) -> AppState {
    state.with_advisory_transport_factory(Arc::new(move |endpoint| {
        assert_eq!(endpoint, ENDPOINT);
        stub.clone()
    }))
}
pub async fn configure(state: &AppState) {
    for (name, value) in [("advisory.model", "synthetic-model:1"), ("advisory.endpoint", ENDPOINT)] {
        setting_authoring::put(state, name, json!(value)).await.unwrap();
    }
}
pub async fn ready(stub: Arc<Stub>) -> AppState {
    let state = inject(bare().await, stub);
    configure(&state).await;
    state
}
/// A Task written straight to the store, with whatever extra fields the test needs.
pub async fn seed(state: &AppState, id: &str, status: &str, extra: Value) {
    let parsed = UbuId::parse(id).unwrap();
    let now = state.planning_now();
    let envelope = state
        .envelope_for(
            [(parsed.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    let mut payload = json!({"id":parsed,"title":format!("Synthetic lunar teapot {}",&id[id.len()-1..]),"status":status,
        "provenance":{"created_at":now,"authority_source":"user"}});
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        payload[key] = value;
    }
    ubu_store::queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::object_record::NewObjectRecord {
            id: id.into(),
            object_type: ObjectType::Task.as_str().into(),
            version: 1,
            status: status.into(),
            compartment_label: "synthetic-private-compartment".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
pub fn occurrence() -> Value {
    json!({"occurrence":{"routine_objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01","local_date":"2026-09-29","key":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-29T07:00:00/static/t1"}})
}
pub async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
pub async fn run_with(state: &AppState, fields: Value) -> (StatusCode, Value) {
    let mut body = json!({"schema_version":"ubu.orchestrator.advisory_run.v1"});
    for (key, value) in fields.as_object().cloned().unwrap() {
        body[key] = value;
    }
    request(state, "POST", "/advisory/run", body).await
}
pub async fn clarify(state: &AppState, task_id: Option<&str>) -> Value {
    let fields = match task_id {
        Some(id) => json!({"producer":"clarify","task_id":id}),
        None => json!({"producer":"clarify"}),
    };
    let (status, body) = run_with(state, fields).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
pub async fn answer(state: &AppState, candidate_id: &str, version: u64, answers: Value) -> (StatusCode, Value) {
    request(
        state,
        "POST",
        &format!("/advisory/candidate/{candidate_id}/answer"),
        json!({"observed_version":version,"answers":answers}),
    )
    .await
}
pub async fn candidates(state: &AppState) -> Vec<(String, String, i64)> {
    sqlx::query_as("SELECT advisory_candidate_id, lifecycle_state, version FROM advisory_candidates ORDER BY created_at, advisory_candidate_id")
        .fetch_all(state.inner().store.pool())
        .await
        .unwrap()
}
pub async fn task(state: &AppState, id: &str) -> Value {
    let row = ubu_store::queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    let mut payload: Value = serde_json::from_str(&row.payload_json).unwrap();
    payload["__version"] = row.version.into();
    payload["__status"] = row.status.into();
    payload
}
pub async fn ledger(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
