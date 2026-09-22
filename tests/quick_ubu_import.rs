use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, state::AppState,
};
const NOW: &str = "2026-09-22T09:00:00Z";
fn uid(n: u8) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/quick-ubu/snapshot-small.json")).unwrap()
}
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
struct SnapshotFile(std::path::PathBuf);
impl SnapshotFile {
    fn new(contents: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("p1b18-{}.json", UbuId::new(ObjectType::Task)));
        std::fs::write(&path, contents).unwrap();
        Self(path)
    }
}
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
async fn post(state: &AppState, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import/quick-ubu")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn import(state: &AppState, snapshot: &Value, dry_run: bool) -> Value {
    let file = SnapshotFile::new(&snapshot.to_string());
    let (status, response) = post(state, json!({"snapshot_path":file.0,"dry_run":dry_run})).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response
}
async fn ledger(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM mutation_envelopes")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn object(state: &AppState, source: &str) -> Value {
    let payload:String=sqlx::query_scalar("SELECT payload_json FROM objects WHERE json_extract(payload_json,'$.provenance.source.source_kind')='quick_ubu' AND json_extract(payload_json,'$.provenance.source.source_id')=?")
        .bind(source).fetch_one(state.inner().store.pool()).await.unwrap();
    serde_json::from_str(&payload).unwrap()
}
fn counts(response: &Value, kind: &str, created: u64, updated: u64, unchanged: u64) {
    assert_eq!(
        response[kind],
        json!({"created":created,"updated":updated,"unchanged":unchanged})
    );
}
fn skipped(response: &Value, id: u8, reason: &str) {
    assert!(
        response["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["quick_ubu_id"] == uid(id) && s["reason"] == reason),
        "{response}"
    );
}
#[tokio::test]
async fn first_import_maps_synthetic_fixture_and_identical_import_writes_nothing() {
    let state = state().await;
    let snapshot = fixture();
    let response = import(&state, &snapshot, false).await;
    println!(
        "P1B18_FIRST_IMPORT_BEGIN\n{}\nP1B18_FIRST_IMPORT_END",
        serde_json::to_string_pretty(&response).unwrap()
    );
    counts(&response, "routines", 5, 0, 0);
    counts(&response, "tasks", 5, 0, 0);
    counts(&response, "preferences", 2, 0, 0);
    assert_eq!(response["objectives_not_imported"], 1);
    assert_eq!(response["schema_version"], "quick-ubu-import/1");
    for (id, reason) in [
        (1, "negative_reminder"),
        (2, "after_reference_missing"),
        (2, "after_self_reference"),
        (2, "negative_after_offset"),
        (12, "past_static_window"),
        (13, "completed"),
        (16, "routine_occurrence"),
        (17, "orphaned_routine_occurrence"),
        (18, "partial_allowed_range"),
        (19, "invalid_allowed_range"),
        (14, "task_after_unsupported"),
        (14, "dependency_not_imported"),
    ] {
        skipped(&response, id, reason);
    }
    let skips = response["skipped"].as_array().unwrap();
    assert_eq!(skips.len(), 15);
    assert!(skips
        .iter()
        .any(|s| s["quick_ubu_id"] == uid(20)
            && s["reason"].as_str().unwrap().starts_with("invalid: ")));
    for reason in ["non_singleton_bundle", "task_not_imported"] {
        assert!(skips.iter().any(|s| s["reason"] == reason));
    }
    let first = object(&state, &uid(1)).await;
    let second = object(&state, &uid(2)).await;
    assert_eq!(first["mode"], "evergreen");
    assert_eq!(first["recurrence"]["timezone"], "America/New_York");
    assert_eq!(first["routine_instance_template"]["placement"], "static");
    assert_eq!(second["routine_instance_template"]["placement"], "planned");
    assert_eq!(
        second["routine_instance_template"]["occupies_capacity"],
        false
    );
    assert_eq!(
        second["routine_instance_template"]["after"],
        json!([{"objective_id":first["id"],"offset_seconds":3600}])
    );
    for (id, kind) in [
        (3, "weekly"),
        (4, "monthly_day"),
        (5, "first_workday_of_month"),
    ] {
        assert_eq!(
            object(&state, &uid(id)).await["recurrence"]["rule"]["kind"],
            kind
        );
    }
    assert_eq!(
        object(&state, &uid(14)).await["blocked_by"],
        json!([object(&state, &uid(11)).await["id"]])
    );
    let before = ledger(&state).await;
    assert_eq!(before, 12);
    let response = import(&state, &snapshot, false).await;
    for (kind, n) in [("routines", 5), ("tasks", 5), ("preferences", 2)] {
        counts(&response, kind, 0, 0, n);
    }
    assert_eq!(ledger(&state).await, before);
    // Advancing the clock cannot rewrite Preference acquired_date/provenance.
    let later = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-22T09:01:00Z").unwrap(),
    ));
    let response = import(&later, &snapshot, false).await;
    counts(&response, "preferences", 0, 0, 2);
    assert_eq!(ledger(&state).await, before);
}
#[tokio::test]
async fn routine_edits_increment_only_the_relevant_version_and_converge() {
    let state = state().await;
    let mut snapshot = fixture();
    import(&state, &snapshot, false).await;
    snapshot["store"]["routines"][uid(1)]["start_time"] = json!("11:30:00");
    let response = import(&state, &snapshot, false).await;
    counts(&response, "routines", 0, 1, 4);
    let routine = object(&state, &uid(1)).await;
    assert_eq!(routine["routine_instance_template"]["template_version"], 2);
    assert_eq!(routine["recurrence"]["schedule_version"], 1);
    let before = ledger(&state).await;
    counts(&import(&state, &snapshot, false).await, "routines", 0, 0, 5);
    assert_eq!(ledger(&state).await, before);
    snapshot["store"]["routines"][uid(3)]["recurrence"]["Weekly"]["weekdays"] =
        json!(["Tue", "Fri"]);
    counts(&import(&state, &snapshot, false).await, "routines", 0, 1, 4);
    let routine = object(&state, &uid(3)).await;
    assert_eq!(routine["recurrence"]["schedule_version"], 2);
    assert_eq!(routine["routine_instance_template"]["template_version"], 1);
}
#[tokio::test]
async fn stale_and_mainline_completed_objects_are_reported_without_mutation() {
    let state = state().await;
    let mut snapshot = fixture();
    import(&state, &snapshot, false).await;
    let original = object(&state, &uid(11)).await;
    snapshot["store"]["tasks"]
        .as_object_mut()
        .unwrap()
        .remove(&uid(11));
    snapshot["store"]["tasks"][uid(15)]["status"] = json!("Done");
    let response = import(&state, &snapshot, false).await;
    for id in [11, 15] {
        assert!(response["stale"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["quick_ubu_id"] == uid(id)));
        assert_eq!(object(&state, &uid(id)).await["status"], "active");
    }
    assert_eq!(object(&state, &uid(11)).await, original);
    let completed = object(&state, &uid(18)).await;
    // Same canonical/store status rewrite made by log_service completion.
    sqlx::query("UPDATE objects SET status='completed', payload_json=json_set(payload_json,'$.status','completed') WHERE id=?")
        .bind(completed["id"].as_str().unwrap()).execute(state.inner().store.pool()).await.unwrap();
    let before = ledger(&state).await;
    let response = import(&state, &snapshot, false).await;
    assert!(response["diverged"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["quick_ubu_id"] == uid(18)));
    assert_eq!(object(&state, &uid(18)).await["status"], "completed");
    assert_eq!(ledger(&state).await, before);
}
#[tokio::test]
async fn dry_run_has_exact_counts_and_never_writes() {
    let state = state().await;
    let response = import(&state, &fixture(), true).await;
    counts(&response, "routines", 5, 0, 0);
    counts(&response, "tasks", 5, 0, 0);
    counts(&response, "preferences", 2, 0, 0);
    assert_eq!(response["dry_run"], true);
    assert_eq!(ledger(&state).await, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM objects")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn malformed_missing_wrong_version_and_timezone_return_named_400() {
    let state = state().await;
    for content in ["not json".into(), json!({"snapshot_version":2}).to_string()] {
        let file = SnapshotFile::new(&content);
        let (status, response) = post(&state, json!({"snapshot_path":file.0})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            response["diagnostics"][0]["code"],
            "invalid_quick_ubu_snapshot"
        );
    }
    let file = SnapshotFile::new(&fixture().to_string());
    let (status, response) = post(
        &state,
        json!({"snapshot_path":file.0,"timezone":"bad timezone"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["diagnostics"][0]["code"], "invalid_timezone");
    let path = file.0.clone();
    drop(file);
    let (status, response) = post(&state, json!({"snapshot_path":path})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response["diagnostics"][0]["code"],
        "invalid_quick_ubu_snapshot"
    );
    let mut snapshot = fixture();
    snapshot["snapshot_version"] = json!(2);
    let file = SnapshotFile::new(&snapshot.to_string());
    let (status, response) = post(&state, json!({"snapshot_path":file.0})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response["diagnostics"][0]["code"],
        "invalid_quick_ubu_snapshot"
    );
}
#[tokio::test]
async fn invalid_routines_skip_cleanly_and_preferences_keep_mainline_enabled_state() {
    let state = state().await;
    let mut snapshot = fixture();
    snapshot["store"]["routines"][uid(2)]["latest_tod"] = json!("11:00:00");
    snapshot["store"]["routines"][uid(5)]["recurrence"] = json!("QuarterlyFirstWorkday");
    let response = import(&state, &snapshot, false).await;
    counts(&response, "routines", 4, 0, 0);
    skipped(&response, 2, "invalid_local_range");
    assert_eq!(
        object(&state, &uid(5)).await["recurrence"]["rule"]["kind"],
        "first_workday_of_quarter"
    );
    let source = format!("{}|{}", uid(31), uid(32));
    let preference = object(&state, &source).await;
    sqlx::query("UPDATE objects SET payload_json=json_set(payload_json,'$.enabled',json('false')) WHERE id=?")
        .bind(preference["id"].as_str().unwrap()).execute(state.inner().store.pool()).await.unwrap();
    let before = ledger(&state).await;
    counts(
        &import(&state, &snapshot, false).await,
        "preferences",
        0,
        0,
        2,
    );
    assert_eq!(ledger(&state).await, before);
}

#[tokio::test]
async fn concurrent_reimports_share_source_identity() {
    let state = state().await;
    let snapshot = fixture();
    let (a, b) = tokio::join!(
        import(&state, &snapshot, false),
        import(&state, &snapshot, false)
    );
    assert_eq!(
        a["tasks"]["created"].as_u64().unwrap() + b["tasks"]["created"].as_u64().unwrap(),
        5
    );
    assert_eq!(
        a["tasks"]["unchanged"].as_u64().unwrap() + b["tasks"]["unchanged"].as_u64().unwrap(),
        5
    );
    assert_eq!(ledger(&state).await, 12);
}
