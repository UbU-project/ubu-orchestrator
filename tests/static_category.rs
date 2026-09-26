use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_orchestrator as orchestrator;
use ubu_orchestrator::{
    build_router, config::ServerConfig, services::planning_service, state::AppState,
};
use ubu_store::{
    models::{
        calendar_record::NewCalendarRecord, log_record::NewLogRecord,
        object_record::NewObjectRecord,
    },
    queries,
};
#[allow(dead_code)]
#[path = "support/advisory_fixture.rs"]
mod review_fixture;

const START: &str = "2026-06-10T15:00:00Z";
const END: &str = "2026-06-10T17:00:00Z";

async fn state() -> AppState {
    let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
    let state = state.with_clock(ubu_orchestrator::planning_time::FixedClock(UbuTimestamp::parse(START).unwrap()));
    store_calendar_window(&state, START, END).await;
    state
}

fn fixed(start: &str, end: &str, capacity: bool) -> Value {
    json!({"static_window":{"start":start,"end":end},"occupies_capacity":capacity})
}

async fn post(state: &AppState, uri: &str, body: Value) -> Value {
    let response = build_router(state.clone())
        .oneshot(json_request(uri, body))
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn generate(state: &AppState) -> Value {
    post(state, "/planning/generate", json!({})).await
}

async fn calendar(state: &AppState) -> Value {
    let response = build_router(state.clone())
        .oneshot(get_request("/calendar/current"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

fn step<'a>(calendar: &'a Value, id: &str) -> &'a Value {
    calendar["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap()
}

// Merging reindexes steps; all other frozen fields must remain byte-identical.
fn without_index(value: &Value) -> Value {
    let mut value = value.clone();
    value.as_object_mut().unwrap().remove("index");
    value
}

fn codes(response: &Value) -> Vec<&str> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn calendar_contains_capacity_noncapacity_and_category_steps() {
    let state = state().await;
    let mut payload = fixed(START, "2026-06-10T15:20:00Z", true);
    payload["duration_estimate"] = json!({"type":"fixed","seconds":9999});
    payload["correlation_groups"] = json!([{"group":"travel","strength":0.5}]);
    payload["tags"] = json!(["commute", "work"]);
    payload["category_tag"] = json!("commute");
    let capacity = admit_task(&state, "Drive to work", payload).await;
    let noncapacity = admit_task(
        &state,
        "Background routine",
        fixed(START, "2026-06-10T15:10:00Z", false),
    )
    .await;
    let dynamic = admit_task(
        &state,
        "Dynamic work",
        json!({"duration_minutes":10,"tags":["work"]}),
    )
    .await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert!(!request.tasks.iter().any(|t| t.id == noncapacity));
    let anchor = request.tasks.iter().find(|t| t.id == capacity).unwrap();
    assert_eq!(anchor.duration, 1200);
    assert!(anchor.duration_estimate.is_none() && anchor.correlation_groups.is_empty());
    assert_eq!(
        anchor.static_anchor.as_ref().unwrap().start,
        timestamp_seconds(START)
    );
    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    let calendar = calendar(&state).await;
    assert_eq!(calendar["steps"], calendar["selected_candidate"]["steps"]);
    let anchored = step(&calendar, &capacity);
    assert_eq!(anchored["start"], timestamp_seconds(START));
    assert_eq!(anchored["end"], timestamp_seconds("2026-06-10T15:20:00Z"));
    assert_eq!(anchored["placement_authority"], "user_override");
    assert_eq!(anchored["occupies_capacity"], true);
    assert_eq!(anchored["category_tag"], "commute");
    assert_eq!(anchored["gcal_color_id"], "7");
    let direct = step(&calendar, &noncapacity);
    assert_eq!(direct["occupies_capacity"], false);
    assert_eq!(direct["static_anchor"], true);
    assert_eq!(direct["placement_authority"], "user_override");
    assert!(
        step(&calendar, &dynamic)["start"].as_u64().unwrap() >= anchored["end"].as_u64().unwrap()
    );
    assert!(step(&calendar, &dynamic).get("category_tag").is_none());
    assert!(step(&calendar, &dynamic).get("gcal_color_id").is_none());
    for candidate in response["alternatives"].as_array().unwrap() {
        assert_eq!(step(candidate, &noncapacity)["occupies_capacity"], false);
    }
    println!(
        "P1B14_CALENDAR_EXCERPT_BEGIN\n{}\nP1B14_CALENDAR_EXCERPT_END",
        serde_json::to_string_pretty(&json!({"steps": calendar["steps"]})).unwrap()
    );
}

#[tokio::test]
async fn edge_crossings_keep_whole_statics_and_dynamic_horizon() {
    let state = state().await;
    let early = admit_task(
        &state,
        "Cross start",
        fixed("2026-06-10T14:50:12Z", "2026-06-10T15:10:00.001Z", true),
    )
    .await;
    let late = admit_task(
        &state,
        "Cross end",
        fixed("2026-06-10T16:50:00Z", "2026-06-10T17:10:01Z", true),
    )
    .await;
    let outside = admit_task(&state, "Outside", fixed(END, "2026-06-10T17:20:00Z", true)).await;
    let before = admit_task(
        &state,
        "Before",
        fixed("2026-06-10T14:30:00Z", START, false),
    )
    .await;
    let dynamic = admit_task(&state, "Inside", json!({"duration_minutes":15})).await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        request.time_window.as_ref().unwrap().start,
        timestamp_seconds("2026-06-10T14:50:12Z")
    );
    assert_eq!(
        request.time_window.as_ref().unwrap().end,
        timestamp_seconds("2026-06-10T17:10:01Z")
    );
    let d = request.tasks.iter().find(|t| t.id == dynamic).unwrap();
    assert_eq!(d.window.as_ref().unwrap().start, timestamp_seconds(START));
    assert_eq!(d.window.as_ref().unwrap().end, timestamp_seconds(END));
    let response = generate(&state).await;
    assert!(!codes(&response).iter().any(|c| c.contains("collision")));
    let cal = calendar(&state).await;
    assert_eq!(
        step(&cal, &early)["end"],
        timestamp_seconds("2026-06-10T15:10:00Z")
    );
    assert_eq!(
        step(&cal, &late)["end"],
        timestamp_seconds("2026-06-10T17:10:01Z")
    );
    assert!(step(&cal, &dynamic)["start"].as_u64().unwrap() >= timestamp_seconds(START));
    assert!(step(&cal, &dynamic)["end"].as_u64().unwrap() <= timestamp_seconds(END));
    assert!(!cal["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["task_id"] == outside || s["task_id"] == before));
}

#[tokio::test]
async fn dynamic_noncapacity_is_excluded_and_empty_kernel_is_bypassed() {
    let state = state().await;
    let unsupported = admit_task(&state, "Unsupported", json!({"occupies_capacity":false})).await;
    let static_id = admit_task(
        &state,
        "Routine",
        fixed(START, "2026-06-10T15:20:00Z", false),
    )
    .await;
    let response = generate(&state).await;
    assert!(response["plan"].is_null());
    assert!(codes(&response).contains(&"no_capacity_tasks_to_plan"));
    assert!(codes(&response).contains(&"non_capacity_dynamic_task_unsupported"));
    assert!(!codes(&response)
        .iter()
        .any(|c| c.eq_ignore_ascii_case("empty_request") || c.contains("Skeleton")));
    admit_task(&state, "Capacity", json!({"duration_minutes":10})).await;
    let response = generate(&state).await;
    assert!(response["plan"].is_object());
    assert!(codes(&response).contains(&"non_capacity_dynamic_task_unsupported"));
    assert!(!codes(&response).contains(&"no_capacity_tasks_to_plan"));
    let cal = calendar(&state).await;
    assert_eq!(step(&cal, &static_id)["occupies_capacity"], false);
    assert!(!cal["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["task_id"] == unsupported));
}

#[tokio::test]
async fn all_static_collisions_are_reported_before_kernel_with_preconditions() {
    let state = state().await;
    let a = admit_task(&state, "A", fixed(START, "2026-06-10T15:20:00Z", true)).await;
    let b = admit_task(
        &state,
        "B",
        fixed("2026-06-10T15:10:00Z", "2026-06-10T15:30:00Z", true),
    )
    .await;
    let c = admit_task(
        &state,
        "C",
        fixed("2026-06-10T16:00:00Z", "2026-06-10T16:20:00Z", true),
    )
    .await;
    let d = admit_task(
        &state,
        "D",
        fixed("2026-06-10T16:10:00Z", "2026-06-10T16:30:00Z", true),
    )
    .await;
    admit_task(
        &state,
        "Blocked",
        json!({"preconditions":{"target":"facts.ready","predicate":"equals","expected":true}}),
    )
    .await;
    let response = generate(&state).await;
    assert!(response["plan"].is_null());
    assert!(codes(&response).contains(&"task_precondition_blocked"));
    assert!(!codes(&response).iter().any(|c| c.contains("Skeleton")));
    let conflicts: Vec<_> = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "static_task_collision")
        .collect();
    assert_eq!(conflicts.len(), 2, "{response}");
    assert!(
        conflicts[0]["message"].as_str().unwrap().contains(&a)
            && conflicts[0]["message"].as_str().unwrap().contains(&b)
    );
    assert!(
        conflicts[1]["message"].as_str().unwrap().contains(&c)
            && conflicts[1]["message"].as_str().unwrap().contains(&d)
    );
}

#[tokio::test]
async fn absent_static_dependencies_bound_dynamic_start_or_exclude_it() {
    let state = state().await;
    let routine = admit_task(
        &state,
        "Prerequisite",
        fixed(START, "2026-06-10T15:30:00Z", false),
    )
    .await;
    let future = admit_task(&state, "Future", fixed(END, "2026-06-10T17:30:00Z", true)).await;
    let bounded = admit_task(
        &state,
        "Bounded",
        json!({"duration_minutes":10,"depends_on":[routine]}),
    )
    .await;
    let excluded = admit_task(
        &state,
        "Excluded",
        json!({"duration_minutes":10,"depends_on":[future]}),
    )
    .await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert!(!request.tasks.iter().any(|t| t.id == excluded));
    let task = request.tasks.iter().find(|t| t.id == bounded).unwrap();
    assert_eq!(
        task.window.as_ref().unwrap().start,
        timestamp_seconds("2026-06-10T15:30:00Z")
    );
    assert!(task.depends_on.is_empty());
    let response = generate(&state).await;
    assert!(codes(&response).contains(&"dependency_outside_horizon"));
    let cal = calendar(&state).await;
    assert!(
        step(&cal, &bounded)["start"].as_u64().unwrap()
            >= timestamp_seconds("2026-06-10T15:30:00Z")
    );
}

#[tokio::test]
async fn other_exclusions_keep_existing_dependency_behavior() {
    let state = state().await;
    let mut prerequisite = fixed(END, "2026-06-10T17:30:00Z", true);
    prerequisite["preconditions"] =
        json!({"target":"facts.ready","predicate":"equals","expected":true});
    let blocked = admit_task(&state, "Blocked static", prerequisite).await;
    let dynamic = admit_task(
        &state,
        "Available",
        json!({"duration_minutes":10,"depends_on":[blocked]}),
    )
    .await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let task = request
        .tasks
        .iter()
        .find(|task| task.id == dynamic)
        .unwrap();
    assert!(task.depends_on.is_empty());
    assert_eq!(
        task.window.as_ref().unwrap().start,
        timestamp_seconds(START)
    );
    let response = generate(&state).await;
    assert!(response["plan"].is_object());
    assert!(codes(&response).contains(&"task_precondition_blocked"));
    assert!(!codes(&response).contains(&"dependency_outside_horizon"));
}

#[tokio::test]
async fn static_precedence_conflicts_include_absent_noncapacity_prerequisites() {
    let state = state().await;
    let future = admit_task(
        &state,
        "Future routine",
        fixed(END, "2026-06-10T17:30:00Z", false),
    )
    .await;
    let mut dependent = fixed(START, "2026-06-10T15:15:00Z", true);
    dependent["depends_on"] = json!([future]);
    let dependent = admit_task(&state, "Premature", dependent).await;
    let response = generate(&state).await;
    assert!(response["plan"].is_null());
    let conflicts: Vec<_> = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "static_task_collision")
        .collect();
    assert_eq!(conflicts.len(), 1);
    let message = conflicts[0]["message"].as_str().unwrap();
    assert!(message.contains(&future) && message.contains(&dependent));
}

#[tokio::test]
async fn repair_carries_direct_steps_and_frozen_steps_win() {
    let state = state().await;
    admit_task(&state, "Capacity", json!({"duration_minutes":10})).await;
    let mut payload = fixed(START, "2026-06-10T15:10:00Z", false);
    payload["tags"] = json!(["commute"]);
    payload["category_tag"] = json!("commute");
    let frozen = admit_task(&state, "Frozen routine", payload).await;
    let direct = admit_task(
        &state,
        "Later routine",
        fixed("2026-06-10T15:30:00Z", "2026-06-10T15:40:00Z", false),
    )
    .await;
    generate(&state).await;
    let prior = calendar(&state).await;
    let frozen_step = step(&prior, &frozen).clone();
    append_user_override_log(&state, &frozen).await;
    let response = post(
        &state,
        "/planning/recalculate",
        json!({
            "triggered_at":"2026-06-10T15:10:00Z","trigger_type":"worker_request","objects":[]
        }),
    )
    .await;
    assert!(response["plan"].is_object(), "{response}");
    let cal = calendar(&state).await;
    assert_eq!(
        without_index(step(&cal, &frozen)),
        without_index(&frozen_step)
    );
    assert_eq!(step(&cal, &direct)["occupies_capacity"], false);
    assert_eq!(
        cal["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["task_id"] == frozen)
            .count(),
        1
    );
    // Even an explicitly supplied same-id direct step cannot overwrite frozen metadata.
    let prior_plan = planning_service::latest_admitted_plan(&state)
        .await
        .unwrap()
        .unwrap();
    let mut context = planning_service::build_repair_request_from_store(
        &state,
        &prior_plan,
        ubu_orchestrator::api::planning::RepairScopeBody::RemainingWindow,
        vec![],
        &[],
    )
    .await
    .unwrap();
    context
        .non_capacity_tasks
        .iter_mut()
        .find(|t| t.id == frozen)
        .unwrap()
        .window
        .as_mut()
        .unwrap()
        .end += 1;
    let repaired = ubu_planning_core::repair(
        planning_service::repair_kernel_request(&context.request),
        &ubu_planning_cpu::CpuStrategy,
    );
    let stored = planning_service::persist_repair_plan(
        &state,
        &context,
        &repaired.repaired_plan.unwrap(),
        &prior_plan,
        vec![serde_json::from_value(frozen_step.clone()).unwrap()],
    )
    .await
    .unwrap();
    let stored = serde_json::to_value(stored).unwrap();
    assert_eq!(
        without_index(step(&stored, &frozen)),
        without_index(&frozen_step)
    );
}

#[tokio::test]
async fn category_has_no_tag_fallback_and_unmapped_category_is_retained() {
    let state = state().await;
    let none = admit_task(
        &state,
        "Uncategorized",
        json!({"tags":["work"],"duration_minutes":10}),
    )
    .await;
    let unmapped = admit_task(
        &state,
        "Unmapped",
        json!({"tags":["custom","work"],"category_tag":"custom","duration_minutes":10}),
    )
    .await;
    generate(&state).await;
    let cal = calendar(&state).await;
    assert!(step(&cal, &none).get("gcal_color_id").is_none());
    assert!(step(&cal, &none).get("category_tag").is_none());
    assert_eq!(step(&cal, &unmapped)["category_tag"], "custom");
    assert!(step(&cal, &unmapped).get("gcal_color_id").is_none());
}

#[tokio::test]
async fn palette_override_merges_defaults_and_matches_case_exactly() {
    let path = std::env::temp_dir().join(format!(
        "p1b14-palette-{}.json",
        UbuId::new(ObjectType::Task)
    ));
    std::fs::write(&path, r#"{"commute":"6","Commute":"8","custom":"6"}"#).unwrap();
    let state = AppState::in_memory(ServerConfig::from_env().with_category_palette_path(&path))
        .await
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let state = state.with_clock(ubu_orchestrator::planning_time::FixedClock(UbuTimestamp::parse(START).unwrap()));
    store_calendar_window(&state, START, END).await;
    for (category, expected) in [
        ("commute", "6"),
        ("Commute", "8"),
        ("custom", "6"),
        ("work", "9"),
    ] {
        admit_task(
            &state,
            category,
            json!({"tags":[category],"category_tag":category,"duration_minutes":10}),
        )
        .await;
        assert_eq!(
            state.inner().category_palette.color(Some(category)),
            Some(expected)
        );
    }
    generate(&state).await;
    let cal = calendar(&state).await;
    for step in cal["steps"].as_array().unwrap() {
        assert!(step.get("gcal_color_id").is_none());
        assert!(state.inner().category_palette.color(step["category_tag"].as_str()).is_some());
    }
}

#[tokio::test]
async fn invalid_palette_is_a_startup_configuration_error_naming_entry() {
    for invalid in [json!("0"), json!("12"), json!(7), json!("07"), json!(null)] {
        let path = std::env::temp_dir().join(format!(
            "p1b14-invalid-{}.json",
            UbuId::new(ObjectType::Task)
        ));
        std::fs::write(&path, json!({"bad_category":invalid}).to_string()).unwrap();
        let result =
            AppState::in_memory(ServerConfig::from_env().with_category_palette_path(&path)).await;
        std::fs::remove_file(path).unwrap();
        let error = result.err().expect("invalid palette must fail startup");
        assert!(error.to_string().contains("bad_category"), "{error}");
    }
}

#[tokio::test]
async fn admitted_add_tag_preserves_explicit_category_and_colour() {
    let state = state().await;
    let task = admit_task(&state, "Drive", json!({"tags":["commute"],"category_tag":"commute","duration_estimate":{"type":"fixed","seconds":10}})).await;
    let mut candidate = review_fixture::proposal();
    candidate.target_refs[0].id = UbuId::parse(&task).unwrap();
    candidate.normalized_proposal = json!({"operation":"add_tag","tag":"work"});
    let uri = format!(
        "/advisory/candidate/{}/admit",
        candidate.advisory_candidate_id.as_str()
    );
    review_fixture::ingest(&state, candidate).await;
    let admitted = post(&state, &uri, json!({"observed_version":1})).await;
    assert_eq!(admitted["task"]["tags"], json!(["commute", "work"]));
    assert_eq!(admitted["task"]["category_tag"], "commute");
    generate(&state).await;
    let cal = calendar(&state).await;
    assert_eq!(step(&cal, &task)["category_tag"], "commute");
    assert!(step(&cal, &task).get("gcal_color_id").is_none());
    assert_eq!(state.inner().category_palette.color(Some("commute")), Some("7"));
}

#[test]
fn step_defaults_capacity_and_omits_absent_category_fields() {
    let step: ubu_orchestrator::api::planning::ScheduledTaskBody = serde_json::from_value(json!({
        "index":0,"task_id":"task","summary":"Task","start":0,"end":1,
        "start_at":"1970-01-01T00:00:00Z","end_at":"1970-01-01T00:00:01Z",
        "depends_on":[],"static_anchor":false,"placement_authority":"planner"
    }))
    .unwrap();
    assert!(step.occupies_capacity);
    let encoded = serde_json::to_value(step).unwrap();
    assert!(encoded.get("category_tag").is_none() && encoded.get("gcal_color_id").is_none());
}

async fn admit_task(state: &AppState, title: &str, extra: Value) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let mut payload = json!({
        "id": id,
        "title": title,
        "status": "active",
        "provenance": {
            "created_at": now,
            "authority_source": "user",
            "source": {
                "source_kind": "test",
                "source_id": title
            }
        }
    });
    let map = payload.as_object_mut().expect("object");
    for (key, value) in extra.as_object().expect("extra object") {
        map.insert(key.clone(), value.clone());
    }

    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            ubu_core::AuthoritySource::User,
            UbuTimestamp::parse(&now).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("task admitted");
    id
}

async fn store_calendar_window(state: &AppState, start: &str, end: &str) {
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: start.to_owned(),
            window_end: end.to_owned(),
            payload: json!({
                "windows": [{"start": start, "end": end}]
            }),
            created_at: "2026-06-10T14:30:00Z".to_owned(),
        },
    )
    .await
    .expect("calendar stored");
}

async fn append_user_override_log(state: &AppState, task_id: &str) {
    let now = UbuTimestamp::now_utc().to_string();
    let envelope = state
        .envelope_for(
            Default::default(),
            ubu_core::AuthoritySource::UserOverride,
            UbuTimestamp::parse(&now).unwrap(),
        )
        .unwrap();
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "decision_recorded".to_owned(),
            object_refs: json!([task_id]),
            payload: json!({"action": "override"}),
            provenance: json!({
                "created_at": now,
                "authority_source": "user_override"
            }),
            created_at: now,
        },
    )
    .await
    .expect("override log");
}

fn timestamp_seconds(value: &str) -> u64 {
    UbuTimestamp::parse(value)
        .expect("timestamp")
        .inner()
        .unix_timestamp() as u64
}

fn json_request(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}
