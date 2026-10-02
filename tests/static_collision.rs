//! P1B-54 §B: a collision between Static Tasks does not cancel the day.
//!
//! Two capacity-occupying Static Tasks that overlap are an impossibility, and a
//! real calendar has them. Until P1B-54 one such pair meant the kernel was never
//! called and there was no Plan. The pair is now one busy span in the request,
//! each Task keeps its own window, and the collision is a warning that names both
//! by title. Every title here is invented.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock,
    services::planning_service, state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-09-22T08:00:00Z";
const COLLISION: &str = "static_task_collision";
const SHARE: &str = "static_tasks_share_committed_time";
const OCCURRENCE: &str = "routine_occurrence_overlaps_commitment";

fn id(n: u8) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d7e{n:02x}")
}
fn instant(time: &str) -> String {
    format!("2026-09-22T{time}:00Z")
}
fn seconds(time: &str) -> u64 {
    UbuTimestamp::parse(instant(time))
        .unwrap()
        .inner()
        .unix_timestamp() as u64
}
fn fixed(start: &str, end: &str) -> Value {
    json!({"static_window":{"start":instant(start),"end":instant(end)}})
}
fn dynamic(minutes: u64) -> Value {
    json!({"duration_estimate":{"type":"fixed","seconds":minutes * 60}})
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn admit(state: &AppState, n: u8, title: &str, fields: Value) {
    let id = id(n);
    let mut payload = json!({"id":id,"title":title,"status":"active","provenance":{"created_at":NOW,"authority_source":"user"}});
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            UbuTimestamp::parse(NOW).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id,
            object_type: ObjectType::Task.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "test".into(),
            payload,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap();
}
async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
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
async fn generate(state: &AppState) -> Value {
    request(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":instant("18:00")}}),
    )
    .await
}
async fn calendar(state: &AppState) -> Value {
    request(state, "GET", "/calendar/current", Value::Null).await
}
fn messages(response: &Value, code: &str) -> Vec<String> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == code)
        .map(|d| d["message"].as_str().unwrap().to_owned())
        .collect()
}
fn codes(response: &Value) -> Vec<String> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}
/// `(start, end, static)` of a Task's step in the Plan, as instants.
fn window(steps: &Value, n: u8) -> (String, String, bool) {
    let step = steps
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id(n))
        .unwrap_or_else(|| panic!("no step for {}", id(n)));
    (
        step["start_at"].as_str().unwrap().to_owned(),
        step["end_at"].as_str().unwrap().to_owned(),
        step["static_anchor"].as_bool().unwrap(),
    )
}
fn fixed_window(start: &str, end: &str) -> (String, String, bool) {
    (instant(start), instant(end), true)
}
/// The Dynamic steps that overlap `[start, end)`.
fn dynamic_inside(steps: &Value, start: &str, end: &str) -> Vec<Value> {
    steps
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["static_anchor"] == false)
        .filter(|s| s["start"].as_u64().unwrap() < seconds(end) && s["end"].as_u64().unwrap() > seconds(start))
        .cloned()
        .collect()
}
/// What the kernel is handed: `(id, anchor start, duration)` of every Static anchor.
async fn anchors(state: &AppState) -> Vec<(String, u64, u64)> {
    let request = planning_service::build_request_from_store(state)
        .await
        .unwrap();
    let mut anchors: Vec<_> = request
        .tasks
        .iter()
        .filter_map(|task| task.static_anchor.as_ref().map(|anchor| (task.id.clone(), anchor.start, task.duration)))
        .collect();
    anchors.sort();
    anchors
}
fn no_kernel_refusal(response: &Value) {
    // The kernel's own words when it is handed two anchors it will not seat.
    let all = response["diagnostics"].to_string();
    assert!(!all.contains("Skeleton"), "{response}");
    assert!(!all.contains("static anchor collides"), "{response}");
}

/// A morning that is already full, so the first free moment is the end of whatever follows it.
async fn full_morning(state: &AppState) {
    admit(state, 9, "Synthetic morning shift", fixed("08:00", "12:00")).await;
}

#[tokio::test]
async fn two_overlapping_static_tasks_make_a_plan_a_warning_and_one_busy_span() {
    let state = state().await;
    full_morning(&state).await;
    admit(&state, 1, "Synthetic dentist", fixed("12:00", "12:40")).await;
    admit(&state, 2, "Synthetic school run", fixed("12:20", "13:00")).await;
    admit(&state, 3, "Synthetic: sort the button jar", dynamic(30)).await;
    admit(&state, 4, "Synthetic: repot the plastic fern", dynamic(45)).await;

    let response = generate(&state).await;
    assert_eq!(response["status"], "ok", "{response}");
    assert!(response["plan"].is_object(), "{response}");
    no_kernel_refusal(&response);

    // The warning, whole: both titles and both ids.
    let collisions = messages(&response, COLLISION);
    assert_eq!(
        collisions,
        vec![format!(
            "Static Tasks “Synthetic dentist” (`{}`) and “Synthetic school run” (`{}`) overlap; both keep their fixed windows and stay on the Calendar, and the whole span is busy",
            id(1),
            id(2)
        )]
    );
    println!("P1B54_B_COLLISION={}", collisions[0]);
    assert!(messages(&response, SHARE).is_empty(), "{response}");

    // Both keep their windows.
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("12:00", "12:40"));
    assert_eq!(window(steps, 2), fixed_window("12:20", "13:00"));
    // No Dynamic placement is inside the span, which is the two windows together.
    assert_eq!(dynamic_inside(steps, "12:00", "13:00"), Vec::<Value>::new());
    // And the Dynamic work is all there, from the first free moment: the end of the span.
    let first = window(steps, 3).0.min(window(steps, 4).0);
    assert_eq!(first, instant("13:00"));
    assert_eq!(steps.as_array().unwrap().len(), 5);
    println!(
        "P1B54_B_PLAN={}",
        json!(steps.as_array().unwrap().iter().map(|s| json!([s["start_at"], s["end_at"], s["static_anchor"], s["summary"]])).collect::<Vec<_>>())
    );

    // The kernel was handed one anchor for the pair, covering both windows: nothing it refuses.
    assert_eq!(
        anchors(&state).await,
        vec![
            (id(1), seconds("12:00"), 3600),
            (id(9), seconds("08:00"), 4 * 3600)
        ]
    );
    // The stored Plan says the same as the response.
    let stored = calendar(&state).await;
    assert_eq!(window(&stored["steps"], 1), fixed_window("12:00", "12:40"));
    assert_eq!(window(&stored["steps"], 2), fixed_window("12:20", "13:00"));
}

#[tokio::test]
async fn two_static_tasks_with_the_same_window_and_title_are_one_span_and_one_warning() {
    // The shape a store reset leaves behind: the same exported night, captured twice.
    let state = state().await;
    full_morning(&state).await;
    admit(&state, 1, "Synthetic night", fixed("12:00", "14:00")).await;
    admit(&state, 2, "Synthetic night", fixed("12:00", "14:00")).await;
    admit(&state, 3, "Synthetic: sort the button jar", dynamic(30)).await;

    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    no_kernel_refusal(&response);
    assert_eq!(
        messages(&response, COLLISION),
        vec![format!(
            "Static Tasks “Synthetic night” (`{}`) and “Synthetic night” (`{}`) overlap; both keep their fixed windows and stay on the Calendar, and the whole span is busy",
            id(1),
            id(2)
        )]
    );
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("12:00", "14:00"));
    assert_eq!(window(steps, 2), fixed_window("12:00", "14:00"));
    assert_eq!(dynamic_inside(steps, "12:00", "14:00"), Vec::<Value>::new());
    assert_eq!(window(steps, 3), (instant("14:00"), instant("14:30"), false));
}

#[tokio::test]
async fn a_chain_of_overlaps_is_one_span_with_one_warning_for_each_pair() {
    let state = state().await;
    full_morning(&state).await;
    // The first and the third do not overlap each other. The second joins them.
    admit(&state, 1, "Synthetic dentist", fixed("12:00", "12:40")).await;
    admit(&state, 2, "Synthetic school run", fixed("12:30", "13:10")).await;
    admit(&state, 3, "Synthetic kettle descaling", fixed("13:00", "13:45")).await;
    admit(&state, 4, "Synthetic: sort the button jar", dynamic(30)).await;

    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    no_kernel_refusal(&response);
    let collisions = messages(&response, COLLISION);
    assert_eq!(collisions.len(), 2, "{response}");
    assert!(collisions[0].contains(&id(1)) && collisions[0].contains(&id(2)));
    assert!(collisions[1].contains(&id(2)) && collisions[1].contains(&id(3)));
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("12:00", "12:40"));
    assert_eq!(window(steps, 2), fixed_window("12:30", "13:10"));
    assert_eq!(window(steps, 3), fixed_window("13:00", "13:45"));
    assert_eq!(dynamic_inside(steps, "12:00", "13:45"), Vec::<Value>::new());
    assert_eq!(window(steps, 4), (instant("13:45"), instant("14:15"), false));
    assert_eq!(
        anchors(&state).await,
        vec![
            (id(1), seconds("12:00"), 105 * 60),
            (id(9), seconds("08:00"), 4 * 3600)
        ]
    );
}

#[tokio::test]
async fn a_collision_beside_a_contained_task_says_each_thing_once() {
    let state = state().await;
    // The chore is inside the block: "during". The visitor overlaps the block's end: a collision.
    admit(&state, 1, "Synthetic long block", fixed("09:00", "12:00")).await;
    admit(&state, 2, "Synthetic five-minute chore", fixed("10:00", "10:05")).await;
    admit(&state, 3, "Synthetic visitor", fixed("11:30", "12:30")).await;
    admit(&state, 4, "Synthetic: sort the button jar", dynamic(30)).await;

    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    assert_eq!(
        messages(&response, COLLISION),
        vec![format!(
            "Static Tasks “Synthetic long block” (`{}`) and “Synthetic visitor” (`{}`) overlap; both keep their fixed windows and stay on the Calendar, and the whole span is busy",
            id(1),
            id(3)
        )]
    );
    assert_eq!(
        messages(&response, SHARE),
        vec![format!("1 Static Task happens during `{}`; the whole span is busy and every one of them stays on the Calendar", id(1))]
    );
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("09:00", "12:00"));
    assert_eq!(window(steps, 2), fixed_window("10:00", "10:05"));
    assert_eq!(window(steps, 3), fixed_window("11:30", "12:30"));
    assert_eq!(dynamic_inside(steps, "09:00", "12:30"), Vec::<Value>::new());
    // Half an hour fits before the block, from the first whole minute.
    assert_eq!(window(steps, 4), (instant("08:00"), instant("08:30"), false));
}

#[tokio::test]
async fn a_static_dependency_that_cannot_hold_is_dropped_with_the_warning() {
    let state = state().await;
    // The prerequisite is wholly after the Task that depends on it. Nothing overlaps.
    let mut early = fixed("12:00", "12:30");
    early["blocked_by"] = json!([id(2)]);
    admit(&state, 1, "Synthetic departure", early).await;
    admit(&state, 2, "Synthetic packing", fixed("14:00", "14:30")).await;
    admit(&state, 3, "Synthetic: sort the button jar", dynamic(30)).await;

    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    no_kernel_refusal(&response);
    assert_eq!(
        messages(&response, COLLISION),
        vec![format!(
            "Static Task “Synthetic departure” (`{}`) depends on “Synthetic packing” (`{}`), which ends after it starts; both keep their fixed windows and stay on the Calendar, and the dependency is not enforced",
            id(1),
            id(2)
        )]
    );
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("12:00", "12:30"));
    assert_eq!(window(steps, 2), fixed_window("14:00", "14:30"));
    // Two anchors, as they are, and no edge between them.
    assert_eq!(
        anchors(&state).await,
        vec![(id(1), seconds("12:00"), 1800), (id(2), seconds("14:00"), 1800)]
    );
    let step = steps.as_array().unwrap().iter().find(|s| s["task_id"] == id(1)).unwrap();
    assert_eq!(step["depends_on"], json!([]));

    // A pair that overlaps and has such an edge is one warning, and says both things.
    let state = self::state().await;
    let mut early = fixed("12:00", "12:30");
    early["blocked_by"] = json!([id(2)]);
    admit(&state, 1, "Synthetic departure", early).await;
    admit(&state, 2, "Synthetic packing", fixed("12:15", "12:45")).await;
    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    assert_eq!(
        messages(&response, COLLISION),
        vec![format!(
            "Static Task “Synthetic departure” (`{}`) depends on “Synthetic packing” (`{}`), which ends after it starts; both keep their fixed windows and stay on the Calendar, and the dependency is not enforced; the whole span is busy",
            id(1),
            id(2)
        )]
    );
    assert_eq!(anchors(&state).await, vec![(id(1), seconds("12:00"), 45 * 60)]);
}

#[tokio::test]
async fn a_plan_with_no_collision_is_unchanged() {
    let state = state().await;
    // Two Statics that touch and do not overlap, and one that stands alone.
    admit(&state, 1, "Synthetic dentist", fixed("09:00", "09:40")).await;
    admit(&state, 2, "Synthetic school run", fixed("09:40", "10:20")).await;
    admit(&state, 3, "Synthetic kettle descaling", fixed("13:00", "13:45")).await;
    admit(&state, 4, "Synthetic: sort the button jar", dynamic(30)).await;
    admit(&state, 5, "Synthetic: repot the plastic fern", dynamic(45)).await;

    let response = generate(&state).await;
    assert_eq!(response["status"], "ok", "{response}");
    let all = codes(&response);
    assert!(!all.iter().any(|code| code == COLLISION || code == SHARE || code == OCCURRENCE), "{response}");
    // Each Static is its own anchor, exactly as stored: nothing was merged.
    assert_eq!(
        anchors(&state).await,
        vec![
            (id(1), seconds("09:00"), 2400),
            (id(2), seconds("09:40"), 2400),
            (id(3), seconds("13:00"), 2700)
        ]
    );
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("09:00", "09:40"));
    assert_eq!(window(steps, 2), fixed_window("09:40", "10:20"));
    assert_eq!(window(steps, 3), fixed_window("13:00", "13:45"));
    // The Dynamic work is all placed, around the anchors.
    assert_eq!(steps.as_array().unwrap().len(), 5);
    assert!(!window(steps, 4).2 && !window(steps, 5).2);
    for (start, end) in [("09:00", "10:20"), ("13:00", "13:45")] {
        assert_eq!(dynamic_inside(steps, start, end), Vec::<Value>::new());
    }
    // And the same store planned again is the same Plan.
    let again = generate(&state).await;
    let windows = |plan: &Value| plan["plan"]["steps"].as_array().unwrap().iter().map(|s| (s["task_id"].clone(), s["start"].clone(), s["end"].clone())).collect::<Vec<_>>();
    assert_eq!(windows(&again), windows(&response));
}

#[tokio::test]
async fn an_empty_store_still_produces_no_candidates() {
    let state = state().await;
    let response = generate(&state).await;
    assert!(response["plan"].is_null(), "{response}");
    assert!(response.get("selected_candidate").is_none(), "{response}");
    assert_eq!(response["alternatives"], json!([]));
    assert!(!codes(&response).iter().any(|code| code == COLLISION));
    no_kernel_refusal(&response);
}

#[tokio::test]
async fn a_routine_occurrence_over_a_commitment_reads_as_it_did() {
    let state = state().await;
    // A real routine, made the way the app makes one, and a commitment at the time of its
    // occurrence. This pair was planned around before P1B-54, under its own code. Untouched.
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/objective")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "schema_version":"ubu.orchestrator.objective.v1","mode":"evergreen","title":"Synthetic night",
                        "recurrence":{"timezone":"UTC","rule":{"kind":"daily"}},
                        "routine_instance_template":{"title":"Synthetic night","duration_estimate":{"type":"fixed","seconds":7200},"nominal_start":"12:00:00","placement":"static","occupies_capacity":true,"tags":[],"reminder_minutes":[]}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    admit(&state, 2, "Synthetic night", fixed("12:00", "14:00")).await;
    admit(&state, 3, "Synthetic: sort the button jar", dynamic(30)).await;

    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    let steps = &response["plan"]["steps"];
    let occurrence = steps
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["summary"] == "Synthetic night" && s["task_id"] != id(2))
        .unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        messages(&response, OCCURRENCE),
        vec![format!(
            "Routine occurrence `{occurrence}` shares its time with commitment `{}`; both stay on the Calendar and the whole span is busy",
            id(2)
        )]
    );
    assert!(messages(&response, COLLISION).is_empty(), "{response}");
    assert_eq!(window(steps, 2), fixed_window("12:00", "14:00"));
    assert_eq!(dynamic_inside(steps, "12:00", "14:00"), Vec::<Value>::new());

    // With a second commitment at the same time, the two commitments collide with each other,
    // and the occurrence still reads as it did: the shape two store resets leave behind.
    admit(&state, 4, "Synthetic night", fixed("12:00", "14:00")).await;
    let response = generate(&state).await;
    assert!(response["plan"].is_object(), "{response}");
    no_kernel_refusal(&response);
    assert_eq!(
        messages(&response, OCCURRENCE),
        vec![format!(
            "Routine occurrence `{occurrence}` shares its time with commitment `{}`; both stay on the Calendar and the whole span is busy",
            id(2)
        )]
    );
    assert_eq!(
        messages(&response, COLLISION),
        vec![format!(
            "Static Tasks “Synthetic night” (`{}`) and “Synthetic night” (`{}`) overlap; both keep their fixed windows and stay on the Calendar, and the whole span is busy",
            id(2),
            id(4)
        )]
    );
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 2), fixed_window("12:00", "14:00"));
    assert_eq!(window(steps, 4), fixed_window("12:00", "14:00"));
    assert_eq!(dynamic_inside(steps, "12:00", "14:00"), Vec::<Value>::new());
}

#[tokio::test]
async fn a_recalculation_over_a_collision_still_repairs() {
    let state = state().await;
    admit(&state, 1, "Synthetic dentist", fixed("12:00", "12:40")).await;
    admit(&state, 2, "Synthetic school run", fixed("12:20", "13:00")).await;
    admit(&state, 3, "Synthetic: sort the button jar", dynamic(30)).await;
    let first = generate(&state).await;
    assert!(first["plan"].is_object(), "{first}");

    let response = request(
        &state,
        "POST",
        "/planning/recalculate",
        json!({"triggered_at":instant("08:05"),"trigger_type":"worker_request","objects":[]}),
    )
    .await;
    assert!(response["plan"].is_object(), "{response}");
    assert_eq!(messages(&response, COLLISION).len(), 1, "{response}");
    let steps = &response["plan"]["steps"];
    assert_eq!(window(steps, 1), fixed_window("12:00", "12:40"));
    assert_eq!(window(steps, 2), fixed_window("12:20", "13:00"));
    assert_eq!(dynamic_inside(steps, "12:00", "13:00"), Vec::<Value>::new());
}
