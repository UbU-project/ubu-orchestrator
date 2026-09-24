use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::{timestamp_at, FixedClock},
    state::AppState,
};
use ubu_store::{models::calendar_record::NewCalendarRecord, queries};

fn uid(n: u8) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn at(time: &str) -> String {
    format!("2026-09-24T{time}Z")
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(at("07:00:00")).unwrap()))
}
fn routine(n: u8, title: &str, start: &str, duration: i64, latest: &str) -> Value {
    json!({"id":uid(n),"title":title,"recurrence":"Daily","start_time":start,
        "duration":[duration,0],"dynamic":true,"latest_tod":latest})
}
fn meals() -> Vec<Value> {
    let mut routines = vec![
        routine(1, "Breakfast", "08:00:00", 600, "08:10:00"),
        routine(2, "Lunch", "13:00:00", 1800, "13:30:00"),
        routine(3, "Dinner", "19:00:00", 1800, "19:30:00"),
        routine(4, "Brush morning", "08:15:00", 300, "08:20:00"),
        routine(5, "Brush night", "22:00:00", 300, "22:05:00"),
    ];
    for meal in &mut routines[..3] {
        meal["establishes"] = json!(["facts.fed"]);
    }
    routines[3]["requires"] = json!([{"fact":"facts.fed","maximum":[1800,0]}]);
    routines[4]["requires"] = json!([{"fact":"facts.fed","maximum":[10800,0]}]);
    routines[4]["establishes"] = json!(["facts.teeth_clean"]);
    routines
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
    let snapshot = json!({"snapshot_version":1,"store":{"routines":routines,
        "tasks":{},"objectives":{},"bundles":{},"preferences":[]},"task_origins":{}});
    let file = SnapshotFile(
        std::env::temp_dir().join(format!("p1b25-{}.json", UbuId::new(ObjectType::Task))),
    );
    std::fs::write(&file.0, snapshot.to_string()).unwrap();
    post(
        state,
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":"UTC"}),
    )
    .await
}
async fn object(state: &AppState, n: u8) -> Value {
    let payload: String = sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.provenance.source.source_id')=?")
        .bind(uid(n)).fetch_one(state.inner().store.pool()).await.unwrap();
    serde_json::from_str(&payload).unwrap()
}
async fn title(state: &AppState, id: &str) -> String {
    sqlx::query_scalar("SELECT json_extract(payload_json,'$.title') FROM objects WHERE id=?")
        .bind(id)
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
fn resolution(n: u8, target: &str, parent: u8, title: &str) -> Value {
    json!({"quick_ubu_id":uid(n),"target":target,
        "establisher_quick_ubu_id":uid(parent),"establisher_title":title})
}

#[tokio::test]
async fn two_brushes_resolve_to_their_nearest_preceding_meals() {
    let state = state().await;
    let response = import(&state, meals()).await;
    assert_eq!(response["routines"]["created"], 5);
    assert_eq!(response["skipped"], json!([]));
    assert_eq!(
        response["resolved"],
        json!([
            resolution(4, "facts.fed", 1, "Breakfast"),
            resolution(5, "facts.fed", 3, "Dinner"),
        ])
    );
    println!(
        "P1B25_RESOLVED_BEGIN\n{}\nP1B25_RESOLVED_END",
        serde_json::to_string_pretty(&response["resolved"]).unwrap()
    );
    let mut table = String::from("Routine | Establisher | Bounds (seconds)\n");
    for (n, parent, maximum) in [(4, "Breakfast", 1800), (5, "Dinner", 10800)] {
        let child = object(&state, n).await;
        let after = child["routine_instance_template"]["after"]
            .as_array()
            .unwrap();
        assert_eq!(after.len(), 1);
        let predecessor = title(&state, after[0]["objective_id"].as_str().unwrap()).await;
        assert_eq!(predecessor, parent);
        assert_eq!(after[0]["minimum_seconds"], 0);
        assert_eq!(after[0]["maximum_seconds"], maximum);
        assert!(child["routine_instance_template"].get("requires").is_none());
        assert!(child["routine_instance_template"]
            .get("establishes")
            .is_none());
        table.push_str(&format!(
            "{} | {predecessor} | [0..{maximum}]\n",
            child["title"].as_str().unwrap()
        ));
    }
    println!("P1B25_EDGES_BEGIN\n{table}P1B25_EDGES_END");
    let repeated = import(&state, meals()).await;
    assert_eq!(
        repeated["routines"],
        json!({"created":0,"updated":0,"unchanged":5})
    );
    assert_eq!(repeated["resolved"], response["resolved"]);

    // Nominal end wins even when a different routine starts later. A tie at an
    // earlier end must not obscure a unique later end (nor may a future end win).
    let mut variants = meals();
    variants[0]["duration"] = json!([8400, 0]); // 10:20, earlier start but latest end
    variants[0]["latest_tod"] = json!("10:20:00");
    variants[1]["start_time"] = json!("09:00:00");
    variants[1]["latest_tod"] = json!("10:00:00");
    variants[1]["duration"] = json!([3600, 0]);
    variants[2]["start_time"] = json!("09:30:00");
    variants[2]["latest_tod"] = json!("10:00:00");
    variants[3]["start_time"] = json!("10:20:00"); // Equality is preceding.
    variants[3]["latest_tod"] = json!("10:25:00");
    variants[4]["start_time"] = json!("11:00:00");
    variants[4]["latest_tod"] = json!("11:05:00");
    variants[4]["establishes"] = json!(["facts.fed"]);
    variants[4]["requires"] = json!([]);
    let response = import(&state, variants).await;
    assert_eq!(
        response["resolved"],
        json!([resolution(4, "facts.fed", 1, "Breakfast")])
    );
}

#[tokio::test]
async fn sleep_chains_from_night_brush_and_the_whole_day_plans_inside_bounds() {
    let state = state().await;
    let mut routines = meals();
    let mut sleep = routine(6, "Sleep", "22:10:00", 1800, "22:40:00");
    sleep["requires"] = json!([{"fact":"facts.teeth_clean","maximum":[1800,0]}]);
    routines.push(sleep);
    let imported = import(&state, routines).await;
    assert_eq!(imported["routines"]["created"], 6);
    assert_eq!(imported["skipped"], json!([]));
    assert_eq!(
        imported["resolved"][2],
        resolution(6, "facts.teeth_clean", 5, "Brush night")
    );
    let brush = object(&state, 5).await;
    let sleep = object(&state, 6).await;
    assert_eq!(
        sleep["routine_instance_template"]["after"],
        json!([
            {"objective_id":brush["id"],"minimum_seconds":0,"maximum_seconds":1800}
        ])
    );
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: at("07:00:00"),
            window_end: at("23:59:00"),
            payload: json!({"windows":[{"start":at("07:00:00"),"end":at("23:59:00")}]}),
            created_at: at("07:00:00"),
        },
    )
    .await
    .unwrap();
    let generated = post(
        &state,
        "/planning/generate",
        json!({"horizon":{"start":at("07:00:00"),"end":at("23:59:00")}}),
    )
    .await;
    assert_eq!(generated["status"], "ok", "{generated}");
    assert_eq!(generated["unplaced_tasks"], json!([]));
    let steps = generated["plan"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 6, "{generated}");
    let mut lines = String::from("PROBE[plan] status=\"ok\"\n");
    for (step, expected) in steps.iter().zip([
        "Breakfast",
        "Brush morning",
        "Lunch",
        "Dinner",
        "Brush night",
        "Sleep",
    ]) {
        assert_eq!(
            title(&state, step["task_id"].as_str().unwrap()).await,
            expected
        );
        lines.push_str(&format!(
            "PROBE[plan] {} {expected}\n",
            timestamp_at(step["start"].as_u64().unwrap()).unwrap()
        ));
    }
    for pair in steps.windows(2) {
        assert!(pair[0]["end"].as_u64().unwrap() <= pair[1]["start"].as_u64().unwrap());
    }
    let brush_end = steps[4]["end"].as_u64().unwrap();
    let sleep_start = steps[5]["start"].as_u64().unwrap();
    assert!((brush_end..=brush_end + 1800).contains(&sleep_start));
    println!("P1B25_PLAN_BEGIN\n{lines}P1B25_PLAN_END");
}

#[tokio::test]
async fn each_failure_drops_only_its_edge_and_preserves_the_routine() {
    let cases = [
        ("requirement_invalid_target", "fact.fed", None, None),
        ("requirement_invalid_target", "facts.", None, None),
        ("requirement_invalid_target", "facts.a..b", None, None),
        ("requirement_invalid_target", "facts.a b", None, None),
        ("requirement_invalid_target", "facts.café", None, None),
        ("requirement_unestablished", "facts.missing", None, None),
        (
            "requirement_unestablished_before",
            "facts.future",
            None,
            None,
        ),
        ("requirement_ambiguous", "facts.tied", None, None),
        (
            "requirement_fractional_bound",
            "facts.fed",
            Some((0, 1)),
            None,
        ),
        (
            "requirement_fractional_bound",
            "facts.fed",
            None,
            Some((600, 1)),
        ),
        (
            "requirement_inverted_bounds",
            "facts.fed",
            Some((600, 0)),
            Some((300, 0)),
        ),
        (
            "requirement_inverted_bounds",
            "facts.fed",
            None,
            Some((-1, 0)),
        ),
        (
            "requirement_inverted_bounds",
            "facts.fed",
            Some((-1, 0)),
            None,
        ),
    ];
    for (reason, target, offset, maximum) in cases {
        let state = state().await;
        let mut parent = routine(1, "Parent", "08:00:00", 600, "08:10:00");
        parent["establishes"] = json!(["facts.fed", "facts.tied"]);
        let mut tied = routine(2, "Tied parent", "08:05:00", 300, "08:10:00");
        tied["establishes"] = json!(["facts.tied"]);
        let mut future = routine(3, "Future parent", "10:00:00", 600, "10:10:00");
        future["establishes"] = json!(["facts.future"]);
        let mut child = routine(4, "Child", "08:15:00", 300, "08:20:00");
        // The valid existing edge must survive each failure.
        child["after"] = json!([{"template_id":uid(1),"offset":[0,0]}]);
        child["requires"] = json!([{"fact":target,"offset":offset,"maximum":maximum}]);
        let response = import(&state, vec![parent, tied, future, child]).await;
        assert_eq!(response["routines"]["created"], 4, "{response}");
        assert_eq!(
            response["skipped"],
            json!([
                {"kind":"routine","quick_ubu_id":uid(4),"reason":format!("{reason}: {target}")}
            ])
        );
        assert_eq!(response["resolved"], json!([]));
        assert_eq!(
            object(&state, 4).await["routine_instance_template"]["after"],
            json!([
                {"objective_id":object(&state, 1).await["id"],"minimum_seconds":0}
            ])
        );
    }
}

#[tokio::test]
async fn resolved_and_explicit_edges_merge_the_tightest_bounds() {
    for (explicit_min, explicit_max, required_min, required_max, expected_min, expected_max) in [
        (120, Some(600), 60, Some(300), 120, Some(300)),
        (60, Some(300), 120, Some(600), 120, Some(300)),
        (0, None, 0, None, 0, None),
        (0, Some(600), 0, None, 0, Some(600)),
        (0, None, 0, Some(600), 0, Some(600)),
    ] {
        let state = state().await;
        let mut parent = routine(1, "Parent", "08:00:00", 600, "08:10:00");
        let targets = [
            "event_markers.a-b_2",
            "facts.fed",
            "numeric_values.energy",
            "set_memberships.group.member",
        ];
        parent["establishes"] = json!(targets);
        let mut child = routine(2, "Child", "08:15:00", 300, "08:20:00");
        child["after"] = json!([
            {"template_id":uid(1),"offset":[explicit_min,0],"maximum":explicit_max.map(|v| (v,0))},
            {"template_id":uid(1),"offset":[0,0]}
        ]);
        child["requires"] = json!(targets
            .iter()
            .rev()
            .map(|target| json!({
                "fact":target,"offset":[required_min,0],"maximum":required_max.map(|v| (v,0))
            }))
            .collect::<Vec<_>>());
        let response = import(&state, vec![parent, child]).await;
        assert_eq!(response["skipped"], json!([]));
        assert_eq!(
            response["resolved"],
            json!(targets.map(|target| resolution(2, target, 1, "Parent")))
        );
        let mut edge =
            json!({"objective_id":object(&state, 1).await["id"],"minimum_seconds":expected_min});
        if let Some(maximum) = expected_max {
            edge["maximum_seconds"] = json!(maximum);
        }
        assert_eq!(
            object(&state, 2).await["routine_instance_template"]["after"],
            json!([edge])
        );
    }
    // Individually valid declarations can have an empty intersection. Preserve
    // the earlier explicit edge and report the offending requirement at import.
    let state = state().await;
    let mut parent = routine(1, "Parent", "08:00:00", 600, "08:10:00");
    parent["establishes"] = json!(["facts.fed"]);
    let mut child = routine(2, "Child", "08:15:00", 300, "08:20:00");
    child["after"] = json!([{"template_id":uid(1),"offset":[600,0]}]);
    child["requires"] = json!([{"fact":"facts.fed","maximum":[300,0]}]);
    let response = import(&state, vec![parent, child]).await;
    assert_eq!(response["routines"]["created"], 2);
    assert_eq!(response["resolved"], json!([]));
    assert_eq!(
        response["skipped"][0]["reason"],
        "requirement_inverted_bounds: facts.fed"
    );
    assert_eq!(
        object(&state, 2).await["routine_instance_template"]["after"],
        json!([
            {"objective_id":object(&state, 1).await["id"],"minimum_seconds":600}
        ])
    );
}

#[tokio::test]
async fn skipped_establisher_is_unestablished_not_a_dangling_reference() {
    let state = state().await;
    let mut parent = routine(1, "Invalid parent", "08:00:00", 600, "08:10:00");
    parent["duration"] = json!([600, 1]);
    parent["establishes"] = json!(["facts.fed"]);
    let mut child = routine(2, "Child", "08:15:00", 300, "08:20:00");
    child["requires"] = json!([{"fact":"facts.fed"}]);
    let response = import(&state, vec![parent, child]).await;
    assert_eq!(response["routines"]["created"], 1);
    assert_eq!(response["resolved"], json!([]));
    assert_eq!(
        response["skipped"],
        json!([
            {"kind":"routine","quick_ubu_id":uid(1),"reason":"invalid: fractional duration is not representable in whole seconds"},
            {"kind":"routine","quick_ubu_id":uid(2),"reason":"requirement_unestablished: facts.fed"}
        ])
    );
    assert!(object(&state, 2).await["routine_instance_template"]
        .get("after")
        .is_none());
}

#[tokio::test]
async fn establishing_a_required_target_does_not_resolve_to_self() {
    let state = state().await;
    let mut routine = routine(1, "Self", "08:00:00", 600, "08:10:00");
    routine["establishes"] = json!(["facts.fed"]);
    routine["requires"] = json!([{"fact":"facts.fed"}]);
    let response = import(&state, vec![routine]).await;
    assert_eq!(response["routines"]["created"], 1);
    assert_eq!(response["resolved"], json!([]));
    assert_eq!(
        response["skipped"],
        json!([
            {"kind":"routine","quick_ubu_id":uid(1),"reason":"requirement_unestablished: facts.fed"}
        ])
    );
    assert!(object(&state, 1).await["routine_instance_template"]
        .get("after")
        .is_none());
}
