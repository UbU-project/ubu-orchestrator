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
use ubu_store::{
    models::{calendar_record::NewCalendarRecord, object_record::ObjectRecord},
    queries,
};

const NOW: &str = "2026-09-24T08:00:00Z";
const END: &str = "2026-09-24T20:00:00Z";
const PREVIEW: &str = "/projection/calendar/preview";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn bytes(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Vec<u8>) {
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
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
    let (status, bytes) = bytes(state, method, uri, body).await;
    assert!(
        status.is_success(),
        "{uri}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}
async fn preview(state: &AppState) -> Value {
    request(state, "GET", PREVIEW, Value::Null).await
}
async fn capture(state: &AppState, mut fields: Value) -> Value {
    fields["schema_version"] = json!("ubu.orchestrator.task_capture.v1");
    request(state, "POST", "/task", fields).await
}
async fn generate(state: &AppState) -> Value {
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: NOW.into(),
            window_end: END.into(),
            payload: json!({"windows":[{"start":NOW,"end":END}]}),
            created_at: NOW.into(),
        },
    )
    .await
    .unwrap();
    let generated = request(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":END}}),
    )
    .await;
    assert!(generated["plan"].is_object(), "{generated}");
    generated
}
struct SnapshotFile(std::path::PathBuf);
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
async fn import_day(state: &AppState) {
    let mut routines = serde_json::Map::new();
    for (n, title, start, duration, latest, category, transparent, reminders) in [
        (
            1,
            "Breakfast",
            "09:00:00",
            1800,
            "10:00:00",
            "personal",
            false,
            vec![0],
        ),
        (
            2,
            "Work Time",
            "10:00:00",
            7200,
            "13:00:00",
            "work",
            true,
            vec![],
        ),
        (
            3,
            "Standup",
            "14:00:00",
            900,
            "15:00:00",
            "business",
            false,
            vec![10, 0],
        ),
    ] {
        let uid = format!("00000000-0000-4000-8000-{n:012}");
        routines.insert(uid.clone(),json!({"id":uid,"title":title,"recurrence":"Daily","start_time":start,"duration":[duration,0],"dynamic":true,"latest_tod":latest,"category":category,"transparent":transparent,"reminders":reminders}));
    }
    let snapshot = json!({"snapshot_version":1,"store":{"routines":routines,"tasks":{},"objectives":{},"bundles":{},"preferences":[]},"task_origins":{}});
    let file = SnapshotFile(
        std::env::temp_dir().join(format!("p1b28-{}.json", UbuId::new(ObjectType::Task))),
    );
    std::fs::write(&file.0, snapshot.to_string()).unwrap();
    let imported = request(
        state,
        "POST",
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":"UTC"}),
    )
    .await;
    assert_eq!(imported["routines"]["created"], 3, "{imported}");
    assert_eq!(imported["skipped"], json!([]));
}
fn by_title<'a>(body: &'a Value, field: &str, title: &str) -> &'a Value {
    body[field]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["summary"] == title)
        .unwrap()
}

#[tokio::test]
async fn three_routines_project_colors_transparency_reminders_and_start_order() {
    let state = state().await;
    import_day(&state).await;
    generate(&state).await;
    let projected = preview(&state).await;
    assert_eq!(
        projected["schema_version"],
        "ubu.orchestrator.calendar_projection_preview.v1"
    );
    let events = projected["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert!(events
        .windows(2)
        .all(|pair| pair[0]["start_at"].as_str() <= pair[1]["start_at"].as_str()));
    for (title, color, transparent, reminders) in [
        ("Breakfast", Value::Null, false, json!([0])),
        ("Work Time", Value::Null, true, json!([])),
        ("Standup", Value::Null, false, json!([10, 0])),
    ] {
        let event = by_title(&projected, "events", title);
        assert_eq!(event["color_id"], color);
        assert_eq!(event["transparent"], transparent);
        assert_eq!(event["reminders_minutes"], reminders);
        assert_eq!(
            event["external_id"].as_str().unwrap(),
            event["task_id"]
                .as_str()
                .unwrap()
                .strip_prefix("task_")
                .unwrap()
        );
    }
    assert_eq!(projected["diagnostics"], json!([]));
    println!("P1B28_EVENTS {}", projected["events"]);

    // Legacy malformed reminder lists must not prevent a preview. Change only
    // the synthetic template; the already generated occurrence is untouched.
    let row:ObjectRecord=sqlx::query_as("SELECT * FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.title')='Standup'")
        .fetch_one(state.inner().store.pool()).await.unwrap();
    let original: Value = serde_json::from_str(&row.payload_json).unwrap();
    for reminders in [
        None,
        Some(json!(null)),
        Some(json!("bad")),
        Some(json!([10, "bad"])),
        Some(json!([1.5])),
        Some(json!([-1])),
        Some(json!([18446744073709551615_u64])),
    ] {
        let mut malformed = original.clone();
        if let Some(reminders) = reminders {
            malformed["routine_instance_template"]["reminder_minutes"] = reminders;
        } else {
            malformed["routine_instance_template"]
                .as_object_mut()
                .unwrap()
                .remove("reminder_minutes");
        }
        sqlx::query("UPDATE objects SET payload_json=? WHERE id=?")
            .bind(malformed.to_string())
            .bind(&row.id)
            .execute(state.inner().store.pool())
            .await
            .unwrap();
        let projected = preview(&state).await;
        assert_eq!(
            by_title(&projected, "events", "Standup")["reminders_minutes"],
            json!([])
        );
    }
}

#[tokio::test]
async fn transparent_mandatory_routine_preserves_capacity_in_step_and_event() {
    let state = state().await;
    import_day(&state).await;
    let generated = generate(&state).await;
    let step = by_title(&generated["plan"], "steps", "Work Time");
    let stored = queries::get_current_state(
        state.inner().store.pool(),
        step["task_id"].as_str().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let task: Value = serde_json::from_str(&stored.payload_json).unwrap();
    println!(
        "PROBE task  `Work Time` occupies_capacity={}",
        task["occupies_capacity"]
    );
    println!(
        "PROBE step  `Work Time` occupies_capacity={}",
        step["occupies_capacity"]
    );
    assert_eq!(task["occupies_capacity"], false);
    assert_eq!(step["occupies_capacity"], false);
    let projected = preview(&state).await;
    assert_eq!(
        by_title(&projected, "events", "Work Time")["transparent"],
        true
    );
}

#[tokio::test]
async fn captured_one_off_has_no_reminders_or_category_color() {
    let state = state().await;
    let captured = capture(
        &state,
        json!({"title":"Call the plumber","duration_estimate":{"type":"fixed","seconds":900}}),
    )
    .await;
    generate(&state).await;
    let projected = preview(&state).await;
    assert_eq!(projected["events"].as_array().unwrap().len(), 1);
    let event = by_title(&projected, "events", "Call the plumber");
    assert_eq!(event["task_id"], captured["task_id"]);
    assert_eq!(event["reminders_minutes"], json!([]));
    assert!(event["color_id"].is_null());
    assert_eq!(event["transparent"], false);
}

#[tokio::test]
async fn preview_preserves_current_calendar_plan_id_and_staleness() {
    let state = state().await;
    let empty = preview(&state).await;
    let current = request(&state, "GET", "/calendar/current", Value::Null).await;
    assert_eq!(empty["plan_id"], current["plan_id"]);
    assert_eq!(empty["stale"], current["stale"]);
    assert!(empty["plan_id"].is_null());
    assert_eq!(empty["events"], json!([]));
    assert_eq!(empty["operations"], json!([]));
    capture(&state,json!({"title":"Impossible deadline","duration_estimate":{"type":"fixed","seconds":900},"due_at":"2026-09-24T07:00:00Z"})).await;
    generate(&state).await;
    let projected = preview(&state).await;
    let current = request(&state, "GET", "/calendar/current", Value::Null).await;
    assert!(current["plan_id"].is_string());
    assert_eq!(current["stale"], true, "{current}");
    assert_eq!(projected["plan_id"], current["plan_id"]);
    assert_eq!(projected["stale"], current["stale"]);
}

// Snapshot every table except the authorized preview records. Canonical state
// and unrelated projection state must remain unchanged.
async fn database_snapshot(state: &AppState) -> Vec<(String, Vec<String>)> {
    use sqlx::Row;
    let pool = state.inner().store.pool();
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut snapshot = Vec::new();
    for table in tables {
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let columns = sqlx::query(&format!("PRAGMA table_info({quoted})"))
            .fetch_all(pool)
            .await
            .unwrap();
        let values = columns
            .iter()
            .map(|row| {
                let name: String = row.get("name");
                format!("quote(\"{}\")", name.replace('"', "\"\""))
            })
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let rows: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT {values} AS encoded FROM {quoted} ORDER BY encoded"
        ))
        .fetch_all(pool)
        .await
        .unwrap();
        if table != "projection_previews" { snapshot.push((table, rows)); }
    }
    snapshot
}

#[tokio::test]
async fn repeated_preview_preserves_content_and_only_persists_previews() {
    let state = state().await;
    import_day(&state).await;
    generate(&state).await;
    let before = database_snapshot(&state).await;
    let (status, first) = bytes(&state, "GET", PREVIEW, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let (status, second) = bytes(&state, "GET", PREVIEW, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let mut first_content: Value = serde_json::from_slice(&first).unwrap();
    let mut second_content: Value = serde_json::from_slice(&second).unwrap();
    let first_id = first_content.as_object_mut().unwrap().remove("preview_id").unwrap();
    let second_id = second_content.as_object_mut().unwrap().remove("preview_id").unwrap();
    assert_ne!(first_id, second_id);
    assert_eq!(first_content, second_content);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projection_previews WHERE id IN (?, ?)")
        .bind(first_id.as_str().unwrap()).bind(second_id.as_str().unwrap())
        .fetch_one(state.inner().store.pool()).await.unwrap();
    assert_eq!(count, 2);
    assert_eq!(database_snapshot(&state).await, before);
    let projected: Value = serde_json::from_slice(&first).unwrap();
    let ops = projected["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    assert!(ops.iter().all(|op| op["kind"] == "create"));
    assert!(ops
        .windows(2)
        .all(|pair| pair[0]["event"]["external_id"].as_str()
            < pair[1]["event"]["external_id"].as_str()));
    for op in ops {
        assert!(projected["events"]
            .as_array()
            .unwrap()
            .contains(&op["event"]));
    }

    // A malformed id in a legacy Plan must be diagnosed rather than silently
    // dropped. This synthetic persisted Plan deliberately bypasses admission.
    let raw: String = sqlx::query_scalar("SELECT payload_json FROM plans WHERE id=?")
        .bind(projected["plan_id"].as_str().unwrap())
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    let mut plan: Value = serde_json::from_str(&raw).unwrap();
    plan["steps"][0]["task_id"] = json!("task_bad!");
    sqlx::query("UPDATE plans SET payload_json=? WHERE id=?")
        .bind(plan.to_string())
        .bind(projected["plan_id"].as_str().unwrap())
        .execute(state.inner().store.pool())
        .await
        .unwrap();
    let invalid = preview(&state).await;
    assert_eq!(invalid["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        invalid["diagnostics"][0]["code"],
        "calendar_event_id_unmappable"
    );
    assert!(invalid["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains("task_bad!"));
}
