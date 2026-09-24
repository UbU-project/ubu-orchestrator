use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::user_action::TASK_ACTION_SCHEMA_VERSION, build_router, config::ServerConfig,
    planning_time::FixedClock, services::planning_service, state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const ROUTINE: &str = "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01";
const COMMITMENT: &str = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d8e02";
const DECLARED: u64 = 600;
const RUNS: [u64; 6] = [900, 960, 1020, 1200, 1500, 1800];
fn timestamp(day: u8, minute: u64) -> String {
    format!("2026-09-{day:02}T09:{minute:02}:00Z")
}
fn at(state: &AppState, time: &str) -> AppState {
    state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(time).unwrap()))
}
async fn post(state: &AppState, uri: &str, body: Value) -> Value {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}
async fn admit(state: &AppState, id: &str, kind: ObjectType, fields: Value) {
    let now = state.planning_now();
    let mut payload = json!({"id":id,"status":"active","provenance":{"created_at":now,"authority_source":"user"}});
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let env = state
        .envelope_for(
            [(UbuId::parse(id).unwrap(), VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &env,
        NewObjectRecord {
            id: id.into(),
            object_type: kind.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "test".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
async fn state() -> AppState {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(timestamp(1, 0)).unwrap()));
    admit(&state, ROUTINE, ObjectType::Objective, json!({"title":"Shower and dress","mode":"evergreen","recurrence":{"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1},"routine_instance_template":{"title":"Morning routine","nominal_start":"09:00:00","placement":"planned","allowed_local_range":{"earliest":"09:00:00","latest":"09:18:00"},"duration_estimate":{"type":"fixed","seconds":DECLARED},"template_version":1}})).await;
    state
}
async fn generate(state: &AppState, day: u8) -> Value {
    let state = at(state, &timestamp(day, 0));
    let response = post(
        &state,
        "/planning/generate",
        json!({"horizon":{"start":timestamp(day,0),"end":format!("2026-09-{day:02}T10:00:00Z")}}),
    )
    .await;
    assert_eq!(response["status"], "ok", "{response}");
    response
}
async fn active_occurrence(state: &AppState) -> String {
    sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=?").bind(ROUTINE).fetch_one(state.inner().store.pool()).await.unwrap()
}
async fn payload(state: &AppState, id: &str) -> Value {
    serde_json::from_str(
        &queries::get_current_state(state.inner().store.pool(), id)
            .await
            .unwrap()
            .unwrap()
            .payload_json,
    )
    .unwrap()
}
async fn action(state: &AppState, id: &str, kind: &str, time: &str) -> Value {
    let response = post(
        &at(state, time),
        &format!("/task/{id}/action"),
        json!({"schema_version":TASK_ACTION_SCHEMA_VERSION,"action":kind}),
    )
    .await;
    let (created_at, log_payload): (String, String) =
        sqlx::query_as("SELECT created_at,payload_json FROM logs WHERE id=?")
            .bind(response["log_id"].as_str().unwrap())
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(
        UbuTimestamp::parse(created_at).unwrap(),
        UbuTimestamp::parse(time).unwrap()
    );
    if kind == "start" {
        assert_eq!(response["transition_applied"], false);
        assert_eq!(response["task_status"], "active");
        assert_eq!(response["authority_source"], "user");
        assert_eq!(
            serde_json::from_str::<Value>(&log_payload).unwrap()["decision"],
            "task_started"
        );
    }
    response
}
async fn six_days() -> AppState {
    let state = state().await;
    for (index, seconds) in RUNS.into_iter().enumerate() {
        let day = index as u8 + 1;
        generate(&state, day).await;
        let id = active_occurrence(&state).await;
        let before = payload(&state, &id).await;
        action(&state, &id, "start", &timestamp(day, 0)).await;
        assert_eq!(
            payload(&state, &id).await,
            before,
            "start must not rewrite the Task or apply effects"
        );
        let completed = action(&state, &id, "complete", &timestamp(day, seconds / 60)).await;
        assert_eq!(completed["transition_applied"], true);
    }
    state
}
fn observed(response: &Value) -> Vec<&Value> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "duration_model_observed")
        .collect()
}
fn duration(response: &Value, id: &str) -> u64 {
    let step = response["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap();
    step["end"].as_u64().unwrap() - step["start"].as_u64().unwrap()
}
async fn commitment(state: &AppState) {
    admit(state,COMMITMENT,ObjectType::Task,json!({"title":"Morning commitment","static_window":{"start":timestamp(7,18),"end":timestamp(7,23)}})).await;
}
fn confidence(response: &Value) -> Value {
    let candidate = &response["selected_candidate"];
    json!({"display_probability":candidate["display_probability"],"coverage.estimate":candidate["coverage"]["estimate"],"boundary.uncovered_mass":candidate["coverage"]["boundaries"][0]["uncovered_mass"]})
}

#[tokio::test]
async fn seventh_day_uses_observations_and_names_the_objective() {
    let state = six_days().await;
    // A damaged legacy timestamp must not make planning fail or become an observation.
    let id: String = sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task' LIMIT 1")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO logs (id,event_type,object_refs_json,payload_json,provenance_json,created_at) VALUES ('synthetic-bad-time','task_started',?,'{}','{}','not-a-timestamp')").bind(json!([id]).to_string()).execute(state.inner().store.pool()).await.unwrap();
    let response = generate(&state, 7).await;
    let id = active_occurrence(&state).await;
    let diagnostics = observed(&response);
    assert_eq!(diagnostics.len(), 1);
    let message="`Shower and dress` is planned from 6 observed runs, not its declared duration (usually 17 min, 30 min at worst, 15 min at best)";
    assert_eq!(diagnostics[0]["message"], message);
    println!("P1B23_MESSAGE {message}");
    let planned = duration(&response, &id);
    assert_ne!(planned, DECLARED);
    assert!((900..=1800).contains(&planned));
    let request = planning_service::build_request_from_store(&at(&state, &timestamp(7, 0)))
        .await
        .unwrap();
    let task = request.tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(task.duration, 1020);
    assert_eq!(
        serde_json::to_value(&task.duration_estimate).unwrap(),
        json!({"type":"shifted_lognormal_p95","min_seconds":900,"mode_seconds":1020,"p95_seconds":1800})
    );
    // Re-encode only this synthetic fixture's actions in the legacy event vocabulary.
    sqlx::query("UPDATE logs SET event_type = CASE json_extract(payload_json,'$.action') WHEN 'start' THEN 'task_started' ELSE 'task_done' END WHERE event_type='decision_recorded' AND json_extract(payload_json,'$.action') IN ('start','complete')")
        .execute(state.inner().store.pool()).await.unwrap();
    let legacy = generate(&state, 7).await;
    assert_eq!(observed(&legacy), diagnostics);
    assert_eq!(duration(&legacy, &id), planned);
    let wide = post(
        &at(&state, &timestamp(7, 0)),
        "/planning/generate",
        json!({"horizon":{"start":timestamp(7,0),"end":"2026-09-08T10:00:00Z"}}),
    )
    .await;
    assert_eq!(wide["status"], "ok", "{wide}");
    assert_eq!(observed(&wide).len(), 1);
    assert_eq!(wide["plan"]["steps"].as_array().unwrap().len(), 2);
    for step in wide["plan"]["steps"].as_array().unwrap() {
        assert_eq!(
            step["end"].as_u64().unwrap() - step["start"].as_u64().unwrap(),
            1020
        );
    }
}

#[tokio::test]
async fn declaration_is_unchanged_after_six_days_and_observed_planning() {
    let state = six_days().await;
    generate(&state, 7).await;
    let expected = json!({"type":"fixed","seconds":DECLARED});
    let stored = payload(&state, ROUTINE).await;
    assert_eq!(
        stored["routine_instance_template"]["duration_estimate"],
        expected
    );
    assert_eq!(
        payload(&state, &active_occurrence(&state).await).await["duration_estimate"],
        expected
    );
    println!(
        "P1B23_DECLARATION {}",
        stored["routine_instance_template"]["duration_estimate"]
    );
}

#[tokio::test]
async fn no_history_keeps_exact_declared_duration() {
    let state = state().await;
    let response = generate(&state, 7).await;
    assert!(observed(&response).is_empty());
    assert_eq!(
        duration(&response, &active_occurrence(&state).await),
        DECLARED
    );
    let one_off = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d8e03";
    admit(
        &state,
        one_off,
        ObjectType::Task,
        json!({"title":"One-off work","duration_estimate":{"type":"fixed","seconds":60}}),
    )
    .await;
    for (verb, minute) in [("start", 0), ("done", 2)] {
        let time = timestamp(7, minute);
        let action = post(
            &at(&state, &time),
            &format!("/task/{one_off}/{verb}"),
            json!({}),
        )
        .await;
        let created_at: String = sqlx::query_scalar("SELECT created_at FROM logs WHERE id=?")
            .bind(action["log_id"].as_str().unwrap())
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
        assert_eq!(
            UbuTimestamp::parse(created_at).unwrap(),
            UbuTimestamp::parse(time).unwrap()
        );
    }
    let response = generate(&state, 7).await;
    assert!(observed(&response).is_empty());
    assert_eq!(duration(&response, one_off), 60);
}

#[tokio::test]
async fn observed_spread_reduces_confidence_at_the_commitment() {
    let baseline = state().await;
    commitment(&baseline).await;
    let before = generate(&baseline, 7).await;
    let state = six_days().await;
    commitment(&state).await;
    let after = generate(&state, 7).await;
    println!("P1B23_BEFORE {}", confidence(&before));
    println!("P1B23_AFTER {}", confidence(&after));
    assert_eq!(confidence(&before)["display_probability"], 1.0);
    assert_eq!(confidence(&before)["coverage.estimate"], 1.0);
    assert_eq!(confidence(&before)["boundary.uncovered_mass"], 0.0);
    let probability = confidence(&after)["display_probability"].as_f64().unwrap();
    let estimate = confidence(&after)["coverage.estimate"].as_f64().unwrap();
    assert!(probability > 0.0 && probability < 1.0);
    assert!(estimate > 0.0 && estimate < 1.0);
    let coverage = &after["selected_candidate"]["coverage"];
    assert_eq!(coverage["below_threshold"], true);
    assert_eq!(coverage["boundaries"][0]["task_id"], COMMITMENT);
    assert!(
        coverage["boundaries"][0]["uncovered_mass"]
            .as_f64()
            .unwrap()
            > 0.0
    );
}
