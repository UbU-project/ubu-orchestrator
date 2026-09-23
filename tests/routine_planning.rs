use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, services::planning_service,
    state::AppState,
};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};
const NOW: &str = "2026-09-22T12:30:00Z";
const DAY: &str = "2026-09-22";
fn uid(n: u16) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn sec(s: &str) -> u64 {
    UbuTimestamp::parse(s).unwrap().inner().unix_timestamp() as u64
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/routines/snapshot-day.json")).unwrap()
}
fn at(state: &AppState, now: &str) -> AppState {
    state
        .clone()
        .with_clock(FixedClock(UbuTimestamp::parse(now).unwrap()))
}
async fn state() -> AppState {
    at(
        &AppState::in_memory(ServerConfig::from_env()).await.unwrap(),
        NOW,
    )
}
struct SnapshotFile(std::path::PathBuf);
impl SnapshotFile {
    fn new(value: &Value) -> Self {
        let path =
            std::env::temp_dir().join(format!("p1b19-{}.json", UbuId::new(ObjectType::Task)));
        std::fs::write(&path, value.to_string()).unwrap();
        Self(path)
    }
}
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
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
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
async fn ok(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
    let (status, body) = request(state, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}
async fn import_zone(state: &AppState, snapshot: &Value, zone: &str) -> Value {
    let file = SnapshotFile::new(snapshot);
    let r = ok(
        state,
        "POST",
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":zone}),
    )
    .await;
    assert!(
        r["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["reason"] == "past_static_window"),
        "{r}"
    );
    r
}
async fn import(state: &AppState, snapshot: &Value) -> Value {
    import_zone(state, snapshot, "America/New_York").await
}
async fn generate_with(state: &AppState, body: Value) -> Value {
    let r = ok(state, "POST", "/planning/generate", body).await;
    assert!(r["plan"].is_object(), "{r}");
    r
}
async fn generate(state: &AppState) -> Value {
    generate_with(state, json!({})).await
}
async fn seeded() -> AppState {
    let state = state().await;
    import(&state, &fixture()).await;
    state
}
fn payload(row: &ObjectRecord) -> Value {
    serde_json::from_str(&row.payload_json).unwrap()
}
async fn source(state: &AppState, n: u16) -> ObjectRecord {
    sqlx::query_as("SELECT * FROM objects WHERE json_extract(payload_json,'$.provenance.source.source_kind')='quick_ubu' AND json_extract(payload_json,'$.provenance.source.source_id')=?").bind(uid(n)).fetch_one(state.inner().store.pool()).await.unwrap()
}
async fn all_occurrences(state: &AppState) -> Vec<ObjectRecord> {
    sqlx::query_as("SELECT * FROM objects WHERE json_extract(payload_json,'$.occurrence') IS NOT NULL ORDER BY id").fetch_all(state.inner().store.pool()).await.unwrap()
}
async fn occurrences(state: &AppState, n: u16, date: &str) -> Vec<ObjectRecord> {
    let id = source(state, n).await.id;
    all_occurrences(state)
        .await
        .into_iter()
        .filter(|r| {
            let p = payload(r);
            p["occurrence"]["routine_objective_id"] == id && p["occurrence"]["local_date"] == date
        })
        .collect()
}
async fn occurrence(state: &AppState, n: u16) -> ObjectRecord {
    let rows = occurrences(state, n, DAY).await;
    rows.into_iter()
        .find(|r| !(r.status == "moot" && payload(r)["moot_reason_code"] == "superseded"))
        .unwrap()
}
async fn current(state: &AppState, id: &str) -> ObjectRecord {
    queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap()
}
fn steps(response: &Value) -> &Vec<Value> {
    response["plan"]["steps"].as_array().unwrap()
}
fn step<'a>(response: &'a Value, id: &str) -> &'a Value {
    steps(response)
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap_or_else(|| panic!("missing {id}: {response}"))
}
fn diagnostics(response: &Value, code: &str) -> Vec<Value> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == code)
        .cloned()
        .collect()
}
fn triage(response: &Value) -> BTreeSet<String> {
    diagnostics(response, "mandatory_occurrence_unplaceable")
        .iter()
        .map(|d| {
            d["message"]
                .as_str()
                .unwrap()
                .split('`')
                .nth(1)
                .unwrap()
                .into()
        })
        .collect()
}
fn no_blocking(response: &Value) {
    assert!(
        response["risk_report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["blocking"] == false),
        "{response}"
    );
}
async fn action(state: &AppState, id: &str, action: &str) -> Value {
    ok(
        state,
        "POST",
        &format!("/task/{id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":action}),
    )
    .await
}
async fn completed_at(state: &AppState, id: &str, now: &str) {
    let old = current(state, id).await;
    let mut p = payload(&old);
    p["status"] = json!("completed");
    let now = UbuTimestamp::parse(now).unwrap();
    let envelope = state
        .envelope_for(
            [(
                UbuId::parse(id).unwrap(),
                VersionRef::Version(old.version as u64),
            )]
            .into_iter()
            .collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.into(),
            object_type: "Task".into(),
            version: old.version,
            status: "completed".into(),
            compartment_label: old.compartment_label,
            payload: p,
            created_at: old.created_at,
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
async fn summaries(state: &AppState) -> Value {
    ok(state, "GET", "/routines", json!({})).await
}
fn summary<'a>(response: &'a Value, title: &str) -> &'a Value {
    response["routines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == title)
        .unwrap()
}
fn assert_window(step: &Value, start: &str, end: &str) {
    assert_eq!(
        (
            step["start"].as_u64().unwrap(),
            step["end"].as_u64().unwrap()
        ),
        (sec(start), sec(end))
    );
}
fn no_other_overlap(response: &Value, ids: &[String], start: &str, end: &str) {
    for step in steps(response) {
        if !ids.iter().any(|id| step["task_id"] == *id) {
            assert!(
                step["end"].as_u64().unwrap() <= sec(start)
                    || step["start"].as_u64().unwrap() >= sec(end),
                "{step}"
            );
        }
    }
}
#[tokio::test]
async fn f1_routine_day_preserves_committed_time_mandatory_order_and_idempotency() {
    let state = seeded().await;
    let r = generate(&state).await;
    println!(
        "P1B19_F1_BEGIN\n{}\nP1B19_F1_END",
        serde_json::to_string_pretty(
            &json!({"diagnostics":r["diagnostics"],"risk_report":r["risk_report"]})
        )
        .unwrap()
    );
    assert_eq!(all_occurrences(&state).await.len(), 10);
    let mut ids = std::collections::BTreeMap::new();
    for n in 101..=110 {
        ids.insert(n, occurrence(&state, n).await.id);
    }
    let second = occurrence(&state, 102).await;
    let p = payload(&second);
    assert_eq!(p["blocked_by"], json!([ids[&101]]));
    assert_eq!(
        p["allowed_time_range"],
        json!({"earliest_start":"2026-09-22T16:00:00Z","latest_finish":"2026-09-22T18:00:00Z"})
    );
    assert_eq!(second.compartment_label, "quick-ubu-import");
    for (n, start, end) in [
        (101, "13:00", "15:45"),
        (102, "16:00", "18:00"),
        (103, "18:00", "21:00"),
    ] {
        let s = step(&r, &ids[&n]);
        assert!(s["start"].as_u64().unwrap() >= sec(&format!("{DAY}T{start}:00Z")));
        assert!(s["end"].as_u64().unwrap() <= sec(&format!("{DAY}T{end}:00Z")));
        assert_eq!(s["occupies_capacity"], true);
    }
    assert!(step(&r, &ids[&102])["depends_on"]
        .as_array()
        .unwrap()
        .contains(&json!(ids[&101])));
    for (n, start, end) in [
        (104, "21:30", "22:00"),
        (105, "22:00", "22:05"),
        (110, "22:05", "22:20"),
        (109, "19:30", "19:40"),
    ] {
        let s = step(&r, &ids[&n]);
        assert_window(
            s,
            &format!("{DAY}T{start}:00Z"),
            &format!("{DAY}T{end}:00Z"),
        );
        assert_eq!(s["occupies_capacity"], true);
    }
    no_other_overlap(
        &r,
        &[ids[&104].clone(), ids[&105].clone(), ids[&110].clone()],
        "2026-09-22T21:30:00Z",
        "2026-09-22T22:20:00Z",
    );
    assert_window(
        step(&r, &ids[&106]),
        "2026-09-23T02:15:00Z",
        "2026-09-23T02:16:00Z",
    );
    assert_window(
        step(&r, &ids[&107]),
        "2026-09-23T02:16:00Z",
        "2026-09-23T02:21:00Z",
    );
    assert_eq!(step(&r, &ids[&108])["occupies_capacity"], false);
    let meeting = source(&state, 201).await.id;
    let backlog = source(&state, 202).await.id;
    assert_window(
        step(&r, &meeting),
        "2026-09-22T19:00:00Z",
        "2026-09-22T20:00:00Z",
    );
    assert!(step(&r, &backlog)["end"].as_u64().unwrap() <= sec("2026-09-22T19:00:00Z"));
    no_other_overlap(
        &r,
        &[meeting, ids[&109].clone()],
        "2026-09-22T19:00:00Z",
        "2026-09-22T20:00:00Z",
    );
    assert!(diagnostics(&r, "static_task_collision").is_empty());
    assert!(!diagnostics(&r, "routine_occurrence_overlaps_commitment").is_empty());
    assert!(diagnostics(&r, "routine_occurrences_overlap").is_empty());
    no_blocking(&r);
    assert!(r["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["category"] == "routine_triage"
            && f["subject_ref"] == ids[&109]
            && f["blocking"] == false));
    assert!(r["task_priorities"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| !ids.values().any(|id| p["task_id"] == *id)));
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    for n in 101..=103 {
        assert_eq!(
            request
                .tasks
                .iter()
                .find(|t| t.id == ids[&n])
                .unwrap()
                .value,
            0.0
        );
    }
    let order = &request.task_graph.unwrap().topological_order;
    let first = order.iter().position(|id| id == &ids[&101]).unwrap();
    assert!(
        first
            < order
                .iter()
                .position(|id| request.tasks.iter().any(|t| &t.id == id && t.value > 0.0))
                .unwrap()
    );
    let before: Vec<_> = all_occurrences(&state)
        .await
        .into_iter()
        .map(|r| (r.id, r.version))
        .collect();
    generate(&state).await;
    assert_eq!(
        before,
        all_occurrences(&state)
            .await
            .into_iter()
            .map(|r| (r.id, r.version))
            .collect::<Vec<_>>()
    );
    let skipped = action(&state, &ids[&109], "skip").await;
    assert_eq!(skipped["task_status"], "moot");
    assert_eq!(skipped["authority_source"], "user");
    assert_eq!(skipped["transition_applied"], true);
    assert!(diagnostics(
        &generate(&state).await,
        "routine_occurrence_overlaps_commitment"
    )
    .is_empty());
    let (status, error) = request_action(&state, &backlog, "skip").await;
    assert_eq!(status, 400);
    assert_eq!(error["diagnostics"][0]["code"], "not_a_routine_occurrence");
    let log: String = sqlx::query_scalar("SELECT payload_json FROM logs WHERE id=?")
        .bind(skipped["log_id"].as_str().unwrap())
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    let log: Value = serde_json::from_str(&log).unwrap();
    assert_eq!(log["decision"], "occurrence_skipped");
    assert_eq!(log["action"], "skip");
    assert_eq!(log["task_status"], "moot");
}
async fn request_action(state: &AppState, id: &str, action: &str) -> (StatusCode, Value) {
    request(
        state,
        "POST",
        &format!("/task/{id}/action"),
        json!({"schema_version":"ubu.orchestrator.task_action.v1","action":action}),
    )
    .await
}
#[tokio::test]
async fn f2_missed_realized_floors_rollups_and_supersession() {
    let state = seeded().await;
    generate(&state).await;
    let one = occurrence(&state, 101).await.id;
    let two = occurrence(&state, 102).await.id;
    let three = occurrence(&state, 103).await.id;
    completed_at(&state, &one, "2026-09-22T15:40:00Z").await;
    let later = at(&state, "2026-09-22T16:00:00Z");
    let r = generate(&later).await;
    assert!(step(&r, &two)["start"].as_u64().unwrap() >= sec("2026-09-22T16:40:00Z"));
    let later = at(&state, "2026-09-22T21:10:00Z");
    let r = generate(&later).await;
    assert_eq!(current(&state, &two).await.status, "failed");
    assert_eq!(current(&state, &three).await.status, "failed");
    assert!(triage(&r).is_empty());
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM logs WHERE event_type='task_failed' AND json_extract(payload_json,'$.routine_outcome')='missed'").fetch_one(state.inner().store.pool()).await.unwrap();
    assert!(count >= 2);
    action(&later, &three, "skip").await;
    let r = summaries(&later).await;
    assert_eq!(summary(&r, "Check-in 1")["done"], 1);
    assert_eq!(summary(&r, "Check-in 1")["current_streak"], 1);
    assert_eq!(summary(&r, "Check-in 2")["missed"], 1);
    assert_eq!(summary(&r, "Check-in 2")["current_streak"], 0);
    assert_eq!(
        summary(&r, "Check-in 3")["last_occurrence"],
        json!({"local_date":DAY,"outcome":"skipped"})
    );
    let old = occurrence(&state, 107).await.id;
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(107)]["start_time"] = json!("22:20:00");
    import(&later, &snapshot).await;
    generate(&later).await;
    let retired = current(&state, &old).await;
    assert_eq!(retired.status, "moot");
    assert_eq!(payload(&retired)["moot_reason_code"], "superseded");
    let new = occurrence(&state, 107).await;
    assert_ne!(new.id, old);
    assert_eq!(new.status, "active");
    assert!(payload(&new)["occurrence"]["key"]
        .as_str()
        .unwrap()
        .ends_with("T22:20:00/static/t2"));
}
#[tokio::test]
async fn f3_repair_drops_skipped_occurrence() {
    let state = seeded().await;
    let r = generate(&state).await;
    let three = occurrence(&state, 103).await.id;
    step(&r, &three);
    action(&state, &three, "skip").await;
    let r=ok(&state,"POST","/planning/recalculate",json!({"schema_version":"ubu.orchestrator.recalculation.v1","triggered_at":NOW,"trigger_type":"task_moot","objects":[{"id":three,"object_type":"Task"}]})).await;
    assert!(r["plan"].is_object(), "{r}");
    assert!(steps(&r).iter().all(|s| s["task_id"] != three), "{r}");
}
#[tokio::test]
async fn f4_late_predecessor_and_closing_window_warn_without_blocking() {
    let state = seeded().await;
    generate(&state).await;
    let one = occurrence(&state, 101).await.id;
    let two = occurrence(&state, 102).await.id;
    let three = occurrence(&state, 103).await.id;
    completed_at(&state, &one, "2026-09-22T17:30:00Z").await;
    let later = at(&state, "2026-09-22T17:31:00Z");
    let r = generate(&later).await;
    assert_eq!(triage(&r), BTreeSet::from([two.clone(), three.clone()]));
    no_blocking(&r);
    action(&later, &two, "skip").await;
    let r = generate(&at(&state, "2026-09-22T20:57:00Z")).await;
    assert_eq!(triage(&r), BTreeSet::from([three]));
    no_blocking(&r);
}
#[tokio::test]
async fn f5_horizon_edge_keeps_next_days_full_range() {
    let state = at(&seeded().await, "2026-09-23T13:02:00Z");
    let r = generate(&state).await;
    let row = occurrences(&state, 101, "2026-09-24").await.pop().unwrap();
    let s = step(&r, &row.id);
    assert!(s["start"].as_u64().unwrap() >= sec("2026-09-24T13:00:00Z"));
    assert!(s["end"].as_u64().unwrap() <= sec("2026-09-24T15:45:00Z"));
    assert_eq!(
        s["end"].as_u64().unwrap() - s["start"].as_u64().unwrap(),
        300
    );
    assert!(triage(&r).is_empty());
}
#[tokio::test]
async fn f6_edit_conflict_late_completion_and_unknown_zone_preserve_history() {
    let state = seeded().await;
    generate(&state).await;
    let old = occurrence(&state, 106).await.id;
    action(&state, &old, "snooze").await;
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(106)]["start_time"] = json!("22:10:00");
    import(&state, &snapshot).await;
    let r = generate(&state).await;
    assert!(!diagnostics(&r, "routine_occurrence_edit_conflict").is_empty());
    let rows = occurrences(&state, 106, DAY).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, old);
    assert_eq!(rows[0].status, "active");
    let later = at(&state, "2026-09-22T16:00:00Z");
    generate(&later).await;
    let one = occurrence(&state, 101).await.id;
    assert_eq!(current(&state, &one).await.status, "failed");
    action(&later, &one, "complete").await;
    let r = summaries(&later).await;
    let s = summary(&r, "Check-in 1");
    assert_eq!(s["done"], 1);
    assert_eq!(s["missed"], 0);
    assert_eq!(s["current_streak"], 1);
    let before = all_occurrences(&state)
        .await
        .into_iter()
        .filter(|r| r.status == "active")
        .count();
    import_zone(&later, &snapshot, "America/Nowhere").await;
    let r = generate(&later).await;
    assert!(!diagnostics(&r, "routine_timezone_unknown").is_empty());
    assert_eq!(
        all_occurrences(&state)
            .await
            .into_iter()
            .filter(|r| r.status == "active")
            .count(),
        before
    );
    let (status, error) = request_action(&later, &one, "skip").await;
    assert_eq!(status, 400);
    assert_eq!(error["diagnostics"][0]["code"], "invalid_task_state");
}
#[tokio::test]
async fn f7_held_days_keep_completed_predecessor_and_reverted_lowering_revives_id() {
    let state = seeded().await;
    generate(&state).await;
    let one = occurrence(&state, 101).await.id;
    let two = occurrence(&state, 102).await.id;
    completed_at(&state, &one, "2026-09-22T15:40:00Z").await;
    let later = at(&state, "2026-09-22T16:00:00Z");
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(101)]["category"] = json!("relationship");
    import(&later, &snapshot).await;
    let r = generate(&later).await;
    let rows: Vec<_> = occurrences(&state, 101, DAY)
        .await
        .into_iter()
        .filter(|r| payload(r)["moot_reason_code"] != "superseded")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, one);
    assert_eq!(rows[0].status, "completed");
    assert_eq!(
        payload(&current(&state, &two).await)["blocked_by"],
        json!([one])
    );
    assert!(step(&r, &two)["start"].as_u64().unwrap() >= sec("2026-09-22T16:40:00Z"));
    let state = seeded().await;
    generate(&state).await;
    let two = occurrence(&state, 102).await.id;
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(101)]["start_time"] = json!("13:00:00");
    snapshot["store"]["routines"][uid(101)]["latest_tod"] = json!("13:30:00");
    import(&state, &snapshot).await;
    let r = generate(&state).await;
    assert!(!diagnostics(&r, "routine_after_infeasible").is_empty());
    assert_eq!(current(&state, &two).await.status, "moot");
    import(&state, &fixture()).await;
    generate(&state).await;
    let revived = current(&state, &two).await;
    assert_eq!(revived.status, "active");
    assert!(payload(&revived).get("moot_reason_code").is_none());
}
fn add_meeting(snapshot: &mut Value, n: u16, start: &str, end: &str) {
    snapshot["store"]["tasks"][uid(n)] = json!({"id":uid(n),"title":"Additional commitment","status":"Backlog","est_duration":[sec(end)-sec(start),0],"pinned":{"start":start,"end":end}});
    snapshot["task_origins"][uid(n)] = json!("manual");
}
#[tokio::test]
async fn f8_no_free_time_is_triaged_and_duplicate_after_is_one_edge() {
    let state = state().await;
    let mut snapshot = fixture();
    add_meeting(
        &mut snapshot,
        203,
        "2026-09-22T13:00:00Z",
        "2026-09-22T15:45:00Z",
    );
    snapshot["store"]["routines"][uid(102)]["after"]
        .as_array_mut()
        .unwrap()
        .push(json!({"template_id":uid(101),"offset":[1800,0]}));
    import(&state, &snapshot).await;
    let r = generate(&state).await;
    no_blocking(&r);
    let one = occurrence(&state, 101).await.id;
    let two = occurrence(&state, 102).await;
    assert_eq!(payload(&two)["blocked_by"], json!([one]));
    let subjects = triage(&r);
    assert!(subjects.contains(&one));
    assert!(subjects.contains(&two.id));
    assert!(diagnostics(&r, "mandatory_occurrence_unplaceable")
        .iter()
        .any(|d| d["message"].as_str().unwrap().contains("no free time")));
}
#[tokio::test]
async fn f9_wide_horizons_legacy_guards_and_read_only_rollups() {
    let state = seeded().await;
    generate_with(
        &state,
        json!({"horizon":{"start":NOW,"end":"2026-09-25T12:30:00Z"}}),
    )
    .await;
    let planning_request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let rows = all_occurrences(&state).await;
    for row in &rows {
        if payload(row)["occurrence"]["local_date"].as_str().unwrap() >= "2026-09-24" {
            assert!(!planning_request.tasks.iter().any(|t| t.id == row.id));
        }
    }
    let r = generate(&state).await;
    let warnings = diagnostics(&r, "routine_occurrence_overlaps_commitment");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0]["message"]
        .as_str()
        .unwrap()
        .contains(&occurrence(&state, 109).await.id));
    let one = occurrence(&state, 101).await.id;
    for verb in ["start", "done", "snooze", "reject", "decompose"] {
        let (status, error) =
            request(&state, "POST", &format!("/task/{one}/{verb}"), json!({})).await;
        assert_eq!(status, 400);
        assert_eq!(error["diagnostics"][0]["code"], "use_recorded_action");
    }
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM mutation_envelopes")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    let r = summaries(&at(&state, "2026-09-22T16:00:00Z")).await;
    assert_eq!(r["schema_version"], "routine-summary/1");
    let s = summary(&r, "Check-in 1");
    assert_eq!(s["missed"], 1);
    assert_eq!(s["pending"], 1);
    assert_eq!(s["last_occurrence"]["outcome"], "missed");
    assert_eq!(current(&state, &one).await.status, "active");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM mutation_envelopes")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(before, after);
    let titles: Vec<_> = r["routines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["title"].as_str().unwrap(),
                r["objective_id"].as_str().unwrap(),
            )
        })
        .collect();
    let mut sorted = titles.clone();
    sorted.sort();
    assert_eq!(titles, sorted);
}
#[tokio::test]
async fn f10_late_event_and_sleep_keep_true_windows_and_reserve_whole_night() {
    let state = state().await;
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(111)] = json!({"id":uid(111),"title":"Sleep routine","recurrence":"Daily","start_time":"23:00:00","duration":[28800,0],"dynamic":false,"transparent":false,"reminders":[0]});
    add_meeting(
        &mut snapshot,
        203,
        "2026-09-23T02:00:00Z",
        "2026-09-23T05:00:00Z",
    );
    import(&state, &snapshot).await;
    let r = generate(&state).await;
    let sleep = occurrence(&state, 111).await.id;
    let event = source(&state, 203).await.id;
    assert_window(
        step(&r, &sleep),
        "2026-09-23T03:00:00Z",
        "2026-09-23T11:00:00Z",
    );
    assert_window(
        step(&r, &event),
        "2026-09-23T02:00:00Z",
        "2026-09-23T05:00:00Z",
    );
    for s in steps(&r)
        .iter()
        .filter(|s| s["placement_authority"] == "planner")
    {
        assert!(
            s["end"].as_u64().unwrap() <= sec("2026-09-23T02:00:00Z")
                || s["start"].as_u64().unwrap() >= sec("2026-09-23T11:00:00Z"),
            "{s}"
        );
    }
    assert!(!diagnostics(&r, "routine_occurrence_overlaps_commitment").is_empty());
    assert!(diagnostics(&r, "static_task_collision").is_empty());
    no_blocking(&r);
}
