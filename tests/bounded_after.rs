use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::{planning::TimeWindowBody, user_action::TASK_ACTION_SCHEMA_VERSION},
    build_router,
    config::ServerConfig,
    planning_time::{timestamp_at, FixedClock},
    services::{
        planning_service,
        routine_instantiation::{instantiate, RoutineDefinition},
    },
    state::AppState,
};
use ubu_store::{
    models::{calendar_record::NewCalendarRecord, object_record::NewObjectRecord},
    queries,
};
const DAY: &str = "2026-09-24";
fn at(time: &str) -> String {
    format!("{DAY}T{time}Z")
}
fn seconds(time: &str) -> u64 {
    UbuTimestamp::parse(at(time))
        .unwrap()
        .inner()
        .unix_timestamp() as u64
}
fn id(n: u8) -> String {
    format!("obj_018f3c8e9b2a7c4d8f1e2a3b4c5d9e{n:02x}")
}
fn edge(n: u8, minimum: i64, maximum: Option<i64>) -> Value {
    let mut edge = json!({"objective_id":id(n),"minimum_seconds":minimum});
    if let Some(maximum) = maximum {
        edge["maximum_seconds"] = json!(maximum);
    }
    edge
}
fn definition(
    n: u8,
    start: &str,
    latest: Option<&str>,
    duration: u64,
    after: Vec<Value>,
) -> RoutineDefinition {
    let mut template = json!({"title":format!("Synthetic routine {n}"),"nominal_start":start,"placement":if latest.is_some(){"planned"}else{"static"},"duration_estimate":{"type":"fixed","seconds":duration},"after":after,"template_version":1});
    if let Some(latest) = latest {
        template["allowed_local_range"] = json!({"earliest":start,"latest":latest});
    }
    RoutineDefinition {
        objective_id: UbuId::parse(id(n)).unwrap(),
        schedule: serde_json::from_value(
            json!({"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1}),
        )
        .unwrap(),
        template: serde_json::from_value(template).unwrap(),
    }
}
async fn state() -> AppState {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(at("21:00:00")).unwrap()));
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: at("21:00:00"),
            window_end: at("23:59:00"),
            payload: json!({"windows":[{"start":at("21:00:00"),"end":at("23:59:00")}]}),
            created_at: at("20:00:00"),
        },
    )
    .await
    .unwrap();
    state
}
async fn admit(state: &AppState, definition: &RoutineDefinition) {
    let now = state.planning_now();
    let env = state
        .envelope_for(
            [(definition.objective_id.clone(), VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(state.inner().store.pool(),&env,NewObjectRecord {id:definition.objective_id.to_string(),object_type:"Objective".into(),version:1,status:"active".into(),compartment_label:"test".into(),payload:json!({"id":definition.objective_id,"title":definition.template.title,"status":"active","mode":"evergreen","recurrence":definition.schedule,"routine_instance_template":definition.template,"provenance":{"created_at":now,"authority_source":"user"}}),created_at:now.to_string(),updated_at:now.to_string()}).await.unwrap();
}
async fn seeded(maximum: Option<i64>, latest: &str) -> AppState {
    let state = state().await;
    admit(
        &state,
        &definition(1, "21:00:00", Some("23:59:00"), 60, vec![]),
    )
    .await;
    admit(
        &state,
        &definition(2, "21:01:00", Some(latest), 1800, vec![edge(1, 0, maximum)]),
    )
    .await;
    state
}
async fn occurrence(state: &AppState, objective: &str) -> String {
    sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=? AND json_extract(payload_json,'$.occurrence.local_date')=?")
        .bind(objective).bind(DAY).fetch_one(state.inner().store.pool()).await.unwrap()
}
async fn window(state: &AppState, objective: &str) -> TimeWindowBody {
    let request = planning_service::build_request_from_store(state)
        .await
        .unwrap();
    let id = occurrence(state, objective).await;
    request
        .tasks
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .window
        .clone()
        .unwrap()
}
fn assert_window(window: &TimeWindowBody, start: &str, end: &str) {
    assert_eq!((window.start, window.end), (seconds(start), seconds(end)));
}
fn wire_window(window: &TimeWindowBody) -> Value {
    json!({"start":timestamp_at(window.start).unwrap(),"end":timestamp_at(window.end).unwrap()})
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
    let response: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{response}");
    response
}
async fn generate(state: &AppState) -> Value {
    post(
        state,
        "/planning/generate",
        json!({"horizon":{"start":at("21:00:00"),"end":at("23:59:00")}}),
    )
    .await
}
fn has(response: &Value, code: &str) -> bool {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}

#[tokio::test]
async fn bounded_and_unbounded_edges_expose_their_kernel_windows() {
    for (maximum, end, label) in [
        (None, "23:59:00", "UNBOUNDED"),
        (Some(600), "21:41:00", "BOUNDED"),
    ] {
        let state = seeded(maximum, "23:59:00").await;
        let window = window(&state, &id(2)).await;
        assert_window(&window, "21:01:00", end);
        println!("P1B24_{label} {}", wire_window(&window));
    }
}

#[tokio::test]
async fn tightest_maximum_and_largest_minimum_win_with_duplicate_edges() {
    let state = state().await;
    let defs = vec![
        definition(1, "21:00:00", Some("23:59:00"), 60, vec![]),
        definition(3, "21:02:00", Some("23:59:00"), 60, vec![]),
        definition(
            2,
            "21:01:00",
            Some("23:59:00"),
            1800,
            vec![
                edge(1, 0, Some(1200)),
                edge(3, 0, Some(300)),
                edge(1, 240, Some(900)),
                edge(1, 60, None),
            ],
        ),
    ];
    for d in &defs {
        admit(&state, d).await;
    }
    assert_window(&window(&state, &id(2)).await, "21:05:00", "21:38:00");
    let expanded = instantiate(&defs, seconds("21:00:00"), seconds("23:59:00"));
    let child = expanded
        .occurrences
        .iter()
        .find(|o| o.objective_id.as_str() == id(2) && o.local_date == DAY)
        .unwrap();
    assert_eq!(child.after.len(), 2);
    assert!(child
        .after
        .contains(&(UbuId::parse(id(1)).unwrap(), 240, Some(900))));
}

#[tokio::test]
async fn contradictory_maximum_has_its_own_diagnostic_and_static_keeps_placement() {
    let state = state().await;
    let parent = definition(1, "21:00:00", Some("23:59:00"), 60, vec![]);
    admit(&state, &parent).await;
    admit(
        &state,
        &definition(
            2,
            "22:00:00",
            Some("23:59:00"),
            1800,
            vec![edge(1, 0, Some(600))],
        ),
    )
    .await;
    let response = generate(&state).await;
    assert!(has(&response, "routine_after_maximum_infeasible"));
    assert!(!has(&response, "routine_after_infeasible"));
    assert!(!response["unplaced_tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["task_id"] == id(2)));
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE json_extract(payload_json,'$.occurrence.routine_objective_id')=?").bind(id(2)).fetch_one(state.inner().store.pool()).await.unwrap();
    assert_eq!(count, 0);
    let static_child = definition(2, "22:00:00", None, 1800, vec![edge(1, 0, Some(600))]);
    let expanded = instantiate(
        &[parent, static_child],
        seconds("21:00:00"),
        seconds("23:59:00"),
    );
    assert!(expanded
        .diagnostics
        .iter()
        .any(|d| d.code == "routine_after_maximum_infeasible"));
    let child = expanded
        .occurrences
        .iter()
        .find(|o| o.objective_id.as_str() == id(2) && o.local_date == DAY)
        .unwrap();
    assert_eq!(
        (child.start, child.end, child.declared_end),
        (
            seconds("22:00:00"),
            seconds("22:30:00"),
            seconds("22:30:00")
        )
    );
}

#[tokio::test]
async fn independently_impossible_chain_retains_the_existing_diagnostic() {
    let state = state().await;
    admit(
        &state,
        &definition(1, "21:00:00", Some("23:59:00"), 60, vec![]),
    )
    .await;
    admit(
        &state,
        &definition(
            2,
            "21:01:00",
            Some("21:40:00"),
            1800,
            vec![edge(1, 3600, Some(7200))],
        ),
    )
    .await;
    let response = generate(&state).await;
    assert!(has(&response, "routine_after_infeasible"));
    assert!(!has(&response, "routine_after_maximum_infeasible"));
}

#[tokio::test]
async fn realized_ceiling_moves_later_to_declared_latest_and_can_move_earlier() {
    for (completion, expected_start, expected_end) in [
        ("21:21:00", "21:21:00", "21:55:00"),
        ("21:00:30", "21:01:00", "21:40:30"),
    ] {
        let state = seeded(Some(600), "21:55:00").await;
        let before = window(&state, &id(2)).await;
        assert_window(&before, "21:01:00", "21:41:00");
        let parent = occurrence(&state, &id(1)).await;
        let later = state
            .clone()
            .with_clock(FixedClock(UbuTimestamp::parse(at(completion)).unwrap()));
        post(
            &later,
            &format!("/task/{parent}/action"),
            json!({"schema_version":TASK_ACTION_SCHEMA_VERSION,"action":"complete"}),
        )
        .await;
        let after = window(&later, &id(2)).await;
        assert_window(&after, expected_start, expected_end);
        if completion == "21:21:00" {
            println!("P1B24_REALIZED_BEFORE {}", wire_window(&before));
            println!("P1B24_REALIZED_AFTER {}", wire_window(&after));
        }
    }
}

struct SnapshotFile(std::path::PathBuf);
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[tokio::test]
async fn imported_maximum_bounds_planning_and_invalid_edges_are_skipped() {
    let state = state().await;
    let uid = |n| format!("00000000-0000-4000-8000-{n:012}");
    let mut routines = serde_json::Map::new();
    for (n, start, duration) in [(1, "21:00:00", 60), (2, "21:01:00", 1800)] {
        routines.insert(uid(n),json!({"id":uid(n),"title":format!("Synthetic {n}"),"recurrence":"Daily","start_time":start,"duration":[duration,0],"dynamic":true,"latest_tod":"23:59:00","transparent":false,"reminders":[],"after":if n==2 {json!([{"template_id":uid(1),"offset":[0,0],"maximum":[600,0]},{"template_id":uid(1),"offset":[600,0],"maximum":[300,0]},{"template_id":uid(1),"offset":[0,0],"maximum":[600,1]}])}else{json!([])}}));
    }
    let snapshot = json!({"snapshot_version":1,"store":{"routines":routines,"tasks":{},"objectives":{},"bundles":{},"preferences":[]},"task_origins":{}});
    let file = SnapshotFile(
        std::env::temp_dir().join(format!("p1b24-{}.json", UbuId::new(ObjectType::Task))),
    );
    std::fs::write(&file.0, snapshot.to_string()).unwrap();
    let imported = post(
        &state,
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":"UTC"}),
    )
    .await;
    for reason in [
        "inverted_after_bounds",
        "invalid: fractional after maximum is not representable in whole seconds",
    ] {
        assert!(
            imported["skipped"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["reason"] == reason),
            "{imported}"
        );
    }
    let payload:String=sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.provenance.source.source_id')=?").bind(uid(2)).fetch_one(state.inner().store.pool()).await.unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let after = &payload["routine_instance_template"]["after"];
    assert_eq!(after.as_array().unwrap().len(), 1);
    assert_eq!(after[0]["minimum_seconds"], 0);
    assert_eq!(after[0]["maximum_seconds"], 600);
    assert!(after[0].get("offset_seconds").is_none());
    let generated = generate(&state).await;
    assert_eq!(generated["status"], "ok", "{generated}");
    assert_window(
        &window(&state, payload["id"].as_str().unwrap()).await,
        "21:01:00",
        "21:41:00",
    );
}
