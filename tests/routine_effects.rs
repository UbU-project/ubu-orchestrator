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
    planning_time::FixedClock, state::AppState,
};
use ubu_store::{
    models::{calendar_record::NewCalendarRecord, object_record::NewObjectRecord},
    queries,
};
fn at(time: &str) -> String {
    format!("2026-09-24T{time}Z")
}
fn uid(n: u8) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn leaf(target: &str) -> Value {
    json!({"target":target,"predicate":"equals","expected":true})
}
fn effect(operation: &str, target: &str) -> Value {
    let mut mutation = json!({"operation":operation,"target":target});
    if operation == "set_fact" {
        mutation["payload"] = json!(true);
    }
    json!({"mutations":[mutation]})
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
            created_at: at("21:00:00"),
        },
    )
    .await
    .unwrap();
    state
}
fn routines(verify: Option<bool>) -> Vec<Value> {
    let mut requirement = json!({"fact":"facts.teeth_clean","maximum":[1800,0]});
    if let Some(verify) = verify {
        requirement["verify"] = json!(verify);
    }
    vec![
        json!({"id":uid(1),"title":"Brush and floss","recurrence":"Daily","start_time":"21:00:00","duration":[300,0],"dynamic":true,"latest_tod":"23:00:00","establishes":["facts.teeth_clean"]}),
        json!({"id":uid(2),"title":"Sleep","recurrence":"Daily","start_time":"21:10:00","duration":[600,0],"dynamic":true,"latest_tod":"23:59:00","requires":[requirement]}),
    ]
}
struct SnapshotFile(std::path::PathBuf);
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn import(state: &AppState, routines: Vec<Value>) -> Value {
    let routines: serde_json::Map<String, Value> = routines
        .into_iter()
        .map(|r| (r["id"].as_str().unwrap().to_owned(), r))
        .collect();
    let snapshot = json!({"snapshot_version":1,"store":{"routines":routines,"tasks":{},"objectives":{},"bundles":{},"preferences":[]},"task_origins":{}});
    let file = SnapshotFile(
        std::env::temp_dir().join(format!("p1b26-{}.json", UbuId::new(ObjectType::Task))),
    );
    std::fs::write(&file.0, snapshot.to_string()).unwrap();
    post(
        state,
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":"UTC"}),
    )
    .await
}
async fn generate(state: &AppState) -> Value {
    post(
        state,
        "/planning/generate",
        json!({"horizon":{"start":state.planning_now(),"end":at("23:59:00")}}),
    )
    .await
}
async fn complete(state: &AppState, task: &Value) -> Value {
    post(
        state,
        &format!("/task/{}/action", task["id"].as_str().unwrap()),
        json!({"schema_version":TASK_ACTION_SCHEMA_VERSION,"action":"complete"}),
    )
    .await
}
async fn object(state: &AppState, object_type: &str, title: &str) -> Value {
    let payload: String = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type=? AND json_extract(payload_json,'$.title')=?")
        .bind(object_type).bind(title).fetch_one(state.inner().store.pool()).await.unwrap();
    serde_json::from_str(&payload).unwrap()
}
async fn universe(state: &AppState) -> Option<Value> {
    let payload: Option<String> = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='UniverseState' ORDER BY updated_at DESC LIMIT 1")
        .fetch_optional(state.inner().store.pool()).await.unwrap();
    payload.map(|p| serde_json::from_str(&p).unwrap())
}
async fn planned(state: &AppState, response: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for step in response["plan"]["steps"].as_array().unwrap() {
        names.push(
            sqlx::query_scalar(
                "SELECT json_extract(payload_json,'$.title') FROM objects WHERE id=?",
            )
            .bind(step["task_id"].as_str().unwrap())
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap(),
        );
    }
    names
}
fn assert_blocked(response: &Value, consumer: &Value) {
    let blocked = response["blocked_tasks"].as_array().unwrap();
    assert_eq!(blocked.len(), 1, "{response}");
    assert_eq!(blocked[0]["task_id"], consumer["id"]);
    assert_eq!(blocked[0]["precondition"], leaf("facts.teeth_clean"));
    assert!(!response["unplaced_tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["task_id"] == consumer["id"]));
}

#[tokio::test]
async fn imported_declarations_lower_to_durable_occurrence_fields() {
    let state = state().await;
    let imported = import(&state, routines(Some(true))).await;
    assert_eq!(imported["skipped"], json!([]));
    generate(&state).await;
    let brush = object(&state, "Task", "Brush and floss").await;
    let sleep = object(&state, "Task", "Sleep").await;
    assert_eq!(brush["effects"], effect("set_fact", "facts.teeth_clean"));
    assert_eq!(sleep["preconditions"], leaf("facts.teeth_clean"));
    println!("P1B26_EFFECTS {}", brush["effects"]);
    println!("P1B26_PRECONDITIONS {}", sleep["preconditions"]);
    generate(&state).await;
    assert_eq!(
        object(&state, "Task", "Brush and floss").await["effects"],
        brush["effects"]
    );
    assert_eq!(
        object(&state, "Task", "Sleep").await["preconditions"],
        sleep["preconditions"]
    );
}

#[tokio::test]
async fn cold_store_seeds_on_completion_and_then_plans_the_consumer() {
    let state = state().await;
    import(&state, routines(Some(true))).await;
    let before = generate(&state).await;
    assert_eq!(planned(&state, &before).await, vec!["Brush and floss"]);
    let brush = object(&state, "Task", "Brush and floss").await;
    let sleep = object(&state, "Task", "Sleep").await;
    assert_blocked(&before, &sleep);
    assert!(universe(&state).await.is_none());
    println!("P1B26_BEFORE facts=(no UniverseState)");
    let later = state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(at("21:05:00")).unwrap()));
    let action = complete(&later, &brush).await;
    assert_eq!(action["diagnostics"], json!([]));
    println!("P1B26_COMPLETE diagnostics={}", action["diagnostics"]);
    let seeded = universe(&state).await.unwrap();
    assert_eq!(seeded["facts"], json!({"teeth_clean":true}));
    assert_eq!(seeded["captured_at"], at("21:05:00"));
    assert_eq!(seeded["provenance"]["authority_source"], "user");
    println!("P1B26_AFTER facts={}", seeded["facts"]);
    let labels: Vec<String> =
        sqlx::query_scalar("SELECT compartment_label FROM objects WHERE id IN (?,?) ORDER BY id")
            .bind(brush["id"].as_str().unwrap())
            .bind(seeded["id"].as_str().unwrap())
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0], labels[1]);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='UniverseState'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
    let after = generate(&later).await;
    assert_eq!(after["status"], "ok", "{after}");
    assert_eq!(planned(&state, &after).await, vec!["Sleep"]);
    assert!(after["blocked_tasks"].is_null() || after["blocked_tasks"] == json!([]));
}

#[tokio::test]
async fn completing_a_destroyer_routine_reblocks_its_consumer() {
    let state = state().await;
    import(&state, routines(Some(true))).await;
    // Quick UbU's establishes syntax only sets true. Mainline template effects
    // express the destroyer without inventing a new Quick UbU declaration.
    let id = UbuId::new(ObjectType::Objective);
    let now = state.planning_now();
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(state.inner().store.pool(),&envelope,NewObjectRecord {
        id:id.to_string(),object_type:"Objective".into(),version:1,status:"active".into(),compartment_label:"test".into(),
        payload:json!({"id":id,"title":"Dinner","status":"active","mode":"evergreen",
            "recurrence":{"timezone":"UTC","rule":{"kind":"daily"},"schedule_version":1},
            "routine_instance_template":{"title":"Dinner","duration_estimate":{"type":"fixed","seconds":60},"nominal_start":"21:06:00","placement":"planned","allowed_local_range":{"earliest":"21:06:00","latest":"23:59:00"},"template_version":1,"effects":effect("clear_fact","facts.teeth_clean")},
            "provenance":{"created_at":now,"authority_source":"user"}}),created_at:now.to_string(),updated_at:now.to_string(),
    }).await.unwrap();
    generate(&state).await;
    let brush = object(&state, "Task", "Brush and floss").await;
    let dinner = object(&state, "Task", "Dinner").await;
    assert_eq!(dinner["effects"], effect("clear_fact", "facts.teeth_clean"));
    let after_brush = state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(at("21:05:00")).unwrap()));
    assert_eq!(
        complete(&after_brush, &brush).await["diagnostics"],
        json!([])
    );
    let ready = generate(&after_brush).await;
    assert!(planned(&state, &ready).await.contains(&"Sleep".to_owned()));
    let after_dinner = state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(at("21:07:00")).unwrap()));
    assert_eq!(
        complete(&after_dinner, &dinner).await["diagnostics"],
        json!([])
    );
    assert!(universe(&state).await.unwrap()["facts"]
        .get("teeth_clean")
        .is_none());
    let blocked = generate(&after_dinner).await;
    let sleep = object(&state, "Task", "Sleep").await;
    assert_blocked(&blocked, &sleep);
    println!("P1B26_REBLOCKED {}", blocked["blocked_tasks"][0]);
}

#[tokio::test]
async fn verification_defaults_false_and_preserves_ordering_without_a_gate() {
    for verify in [None, Some(false)] {
        let state = state().await;
        let imported = import(&state, routines(verify)).await;
        assert_eq!(imported["resolved"].as_array().unwrap().len(), 1);
        let brush = object(&state, "Objective", "Brush and floss").await;
        let sleep = object(&state, "Objective", "Sleep").await;
        assert_eq!(
            sleep["routine_instance_template"]["after"],
            json!([
                {"objective_id":brush["id"],"minimum_seconds":0,"maximum_seconds":1800}
            ])
        );
        assert!(sleep["routine_instance_template"]
            .get("preconditions")
            .is_none());
        let generated = generate(&state).await;
        assert_eq!(generated["status"], "ok");
        assert_eq!(
            planned(&state, &generated).await,
            vec!["Brush and floss", "Sleep"]
        );
        assert!(object(&state, "Task", "Sleep")
            .await
            .get("preconditions")
            .is_none());
        assert!(universe(&state).await.is_none());
    }
}

#[tokio::test]
async fn verified_requirements_form_one_leaf_or_all_of_even_without_resolution() {
    for targets in [
        vec!["facts.teeth_clean"],
        vec!["facts.teeth_clean", "facts.ready"],
    ] {
        let state = state().await;
        let mut routines = routines(Some(true));
        routines[1]["requires"] = json!(targets
            .iter()
            .map(|t| json!({"fact":t,"verify":true}))
            .collect::<Vec<_>>());
        let imported = import(&state, routines).await;
        let expected = if targets.len() == 1 {
            leaf(targets[0])
        } else {
            json!({"all_of":targets.iter().map(|t|leaf(t)).collect::<Vec<_>>()})
        };
        assert_eq!(
            object(&state, "Objective", "Sleep").await["routine_instance_template"]
                ["preconditions"],
            expected
        );
        if targets.len() == 2 {
            assert!(imported["skipped"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["reason"] == "requirement_unestablished: facts.ready"));
        }
        generate(&state).await;
        assert_eq!(
            object(&state, "Task", "Sleep").await["preconditions"],
            expected
        );
    }
}

#[tokio::test]
async fn invalid_establishes_targets_skip_only_the_mutation() {
    let state = state().await;
    let mut routines = routines(None);
    let invalid = [
        "numeric_values.energy",
        "set_memberships.team",
        "event_markers.done",
        "fact.typo",
        "facts.",
        "facts.a..b",
    ];
    routines[0]["establishes"] = json!(std::iter::once("facts.teeth_clean")
        .chain(invalid)
        .collect::<Vec<_>>());
    let imported = import(&state, routines).await;
    assert_eq!(imported["routines"]["created"], 2);
    assert_eq!(imported["skipped"],json!(invalid.map(|t|json!({"kind":"routine","quick_ubu_id":uid(1),"reason":format!("establishes_invalid_target: {t}")}))));
    assert_eq!(
        object(&state, "Objective", "Brush and floss").await["routine_instance_template"]
            ["effects"],
        effect("set_fact", "facts.teeth_clean")
    );
    generate(&state).await;
    assert_eq!(
        object(&state, "Task", "Brush and floss").await["effects"],
        effect("set_fact", "facts.teeth_clean")
    );
}
