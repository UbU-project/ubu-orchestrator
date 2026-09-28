//! Synthetic HTTP contracts in-process; Calendar uses only RecordingCalendarApi.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router,
    category_palette::CategoryPalette,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_client::{CalendarApi, RecordingCalendarApi},
        calendar_projection::DesiredEvent,
    },
    state::AppState,
};
use ubu_store::{models::calendar_record::NewCalendarRecord, queries};
const NOW: &str = "2026-09-28T08:00:00Z";
const END: &str = "2026-09-28T20:00:00Z";
const SCHEMA: &str = "ubu.orchestrator.setting.v1";

async fn setup(config: ServerConfig) -> (AppState, Arc<RecordingCalendarApi>) {
    let recorder = Arc::new(RecordingCalendarApi::new());
    let state = AppState::in_memory(config)
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(recorder.clone());
    (state, recorder)
}
async fn state() -> AppState {
    setup(ServerConfig::from_env()).await.0
}
async fn file_state() -> AppState {
    let path = std::env::temp_dir().join(format!(
        "p1b42-synthetic-palette-{}.json",
        UbuId::new(ObjectType::Setting)
    ));
    std::fs::write(&path, r#"{"work":"6"}"#).unwrap();
    let state = setup(ServerConfig::from_env().with_category_palette_path(&path))
        .await
        .0;
    std::fs::remove_file(path).unwrap(); // The file is a startup seed, not a request-time dependency.
    state
}
async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
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
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)}))
        },
    )
}
async fn ok(state: &AppState, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = request(state, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}
async fn list(state: &AppState) -> Value {
    ok(state, "GET", "/settings", Value::Null).await
}
async fn put(state: &AppState, category: &str, color: &str) -> Value {
    ok(
        state,
        "PUT",
        &format!("/setting/calendar.color.{category}"),
        json!({"schema_version":SCHEMA,"value":color}),
    )
    .await
}
fn entry<'a>(body: &'a Value, category: &str) -> &'a Value {
    body["palette"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["category"] == category)
        .unwrap()
}
async fn count(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Setting'")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap()
}
async fn capture(state: &AppState) -> Value {
    ok(
        state,
        "POST",
        "/projection/calendar/capture",
        json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"}),
    )
    .await
}
async fn generate(state: &AppState) {
    let plan = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":END}}),
    )
    .await;
    assert!(plan["plan"].is_object(), "{plan}");
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", "/projection/calendar/preview", Value::Null).await
}

#[tokio::test]
async fn defaults_have_default_origin_and_no_settings() {
    let s = state().await;
    let body = list(&s).await;
    assert_eq!(body["schema_version"], SCHEMA);
    assert_eq!(body["settings"], json!([]));
    let rows = body["palette"].as_array().unwrap();
    assert_eq!(rows.len(), 11);
    for (category, color) in [
        ("personal", "3"),
        ("relationship", "5"),
        ("business", "6"),
        ("committed", "11"),
        ("location", "8"),
        ("entertainment", "1"),
        ("grocery", "2"),
        ("commute", "7"),
        ("undefined", "4"),
        ("education_house", "10"),
        ("work", "9"),
    ] {
        assert_eq!(
            entry(&body, category),
            &json!({"category":category,"color_id":color,"origin":"default"})
        );
    }
    println!("EVIDENCE[P1B42_test1]={body}");
}
#[tokio::test]
async fn put_admits_then_updates_one_native_setting() {
    let s = state().await;
    let first = put(&s, "work", "6").await;
    let body = list(&s).await;
    assert_eq!(entry(&body, "work")["origin"], "setting");
    assert_eq!(entry(&body, "work")["color_id"], "6");
    let id = first["setting_id"].as_str().unwrap();
    assert!(id.starts_with("setting_"));
    let record = queries::get_current_state(s.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    let payload: Value = serde_json::from_str(&record.payload_json).unwrap();
    assert_eq!(payload["authority_source"], "user");
    assert_eq!(
        payload["provenance"],
        json!({"created_at":NOW,"authority_source":"user"})
    );
    assert!(payload.get("source").is_none());
    assert_eq!(first["version"], 1);
    println!("EVIDENCE[P1B42_test2]={body}");
    let changed = put(&s, "work", "7").await;
    assert_eq!(changed["setting_id"], first["setting_id"]);
    assert_eq!(changed["version"], 2);
    assert_eq!(count(&s).await, 1);
    assert_eq!(entry(&list(&s).await, "work")["color_id"], "7");
}
#[tokio::test]
async fn setting_beats_file_seed_which_beats_default() {
    let s = file_state().await;
    let before = list(&s).await;
    assert_eq!(
        entry(&before, "work"),
        &json!({"category":"work","color_id":"6","origin":"file"})
    );
    assert_eq!(entry(&before, "personal")["origin"], "default");
    put(&s, "work", "2").await;
    let after = list(&s).await;
    assert_eq!(
        entry(&after, "work"),
        &json!({"category":"work","color_id":"2","origin":"setting"})
    );
    println!("EVIDENCE[P1B42_test3_before]={before}");
    println!("EVIDENCE[P1B42_test3_after]={after}");
}
#[tokio::test]
async fn invalid_colours_and_untrusted_or_missing_schema_fields_admit_nothing() {
    let s = state().await;
    for value in [
        json!("0"),
        json!("12"),
        json!("01"),
        json!(1),
        json!(false),
        Value::Null,
    ] {
        let (status, body) = request(
            &s,
            "PUT",
            "/setting/calendar.color.work",
            json!({"schema_version":SCHEMA,"value":value}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["diagnostics"][0]["code"], "setting_invalid_color");
        assert_eq!(body["diagnostics"][0]["message"],"Colour must be a string with one of the allowed ids: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11");
        if value == json!("12") {
            println!("EVIDENCE[P1B42_test4]={body}");
        }
    }
    for (body, code) in [
        (json!({"value":"2"}), "missing_schema_version"),
        (
            json!({"schema_version":"synthetic-wrong","value":"2"}),
            "unknown_schema_version",
        ),
    ] {
        let (status, body) = request(&s, "PUT", "/setting/calendar.color.work", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["diagnostics"][0]["code"], code);
    }
    let (status, _) = request(
        &s,
        "PUT",
        "/setting/calendar.color.work",
        json!({"schema_version":SCHEMA,"value":"2","authority_source":"system"}),
    )
    .await;
    assert!(status.is_client_error());
    assert_eq!(count(&s).await, 0);
}
#[tokio::test]
async fn unknown_namespace_is_rejected_for_put_and_delete() {
    let s = state().await;
    for name in ["work", "calendar.other.work", "calendar.color."] {
        for method in ["PUT", "DELETE"] {
            let (status, body) = request(
                &s,
                method,
                &format!("/setting/{name}"),
                json!({"schema_version":SCHEMA,"value":"2"}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["diagnostics"][0]["code"], "setting_unknown_name");
        }
    }
    assert_eq!(count(&s).await, 0);
}
#[tokio::test]
async fn delete_reverts_file_and_default_and_removes_setting_only_category() {
    for (s, color, origin) in [
        (state().await, "9", "default"),
        (file_state().await, "6", "file"),
    ] {
        put(&s, "work", "2").await;
        let (status, body) =
            request(&s, "DELETE", "/setting/calendar.color.work", Value::Null).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_null());
        let body = list(&s).await;
        assert_eq!(
            entry(&body, "work"),
            &json!({"category":"work","color_id":color,"origin":origin})
        );
        assert_eq!(count(&s).await, 0);
        put(&s, "synthetic-custom", "7").await;
        ok(
            &s,
            "DELETE",
            "/setting/calendar.color.synthetic-custom",
            Value::Null,
        )
        .await;
        assert!(!list(&s).await["palette"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["category"] == "synthetic-custom"));
    }
}
#[tokio::test]
async fn next_preview_uses_setting_without_restart_or_regeneration() {
    let (s, recorder) = setup(ServerConfig::from_env()).await;
    ok(&s,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Synthetic Static appointment","tags":["work"],"category_tag":"work","static_window":{"start":"2026-09-28T10:00:00Z","end":"2026-09-28T10:30:00Z"}})).await;
    generate(&s).await;
    let before = preview(&s).await;
    assert_eq!(before["events"][0]["color_id"], "9");
    put(&s, "work", "2").await;
    let after = preview(&s).await;
    assert_eq!(after["events"][0]["color_id"], "2");
    assert_eq!(after["plan_id"], before["plan_id"]);
    assert_eq!(
        after["events"][0]["task_id"],
        before["events"][0]["task_id"]
    );
    assert!(recorder.recorded_calls().is_empty());
    println!(
        "EVIDENCE[P1B42_test7]={}",
        json!({"before_color_id":before["events"][0]["color_id"],"after_color_id":after["events"][0]["color_id"],"same_state":true,"same_plan":after["plan_id"]==before["plan_id"],"regenerated":false,"restarted":false})
    );
}
#[tokio::test]
async fn inverse_uses_changed_palette_for_capture_and_dynamic_completion_still_works() {
    let (s, recorder) = setup(ServerConfig::from_env()).await;
    put(&s, "work", "1").await;
    put(&s, "entertainment", "9").await;
    let palette = CategoryPalette::from_pool(s.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(palette.inverse().get("1"), Some(&Some("work".into())));
    let foreign = DesiredEvent {
        external_id: "bbbbb".into(),
        task_id: "task_bbbbb".into(),
        summary: "Synthetic foreign appointment".into(),
        start_at: "2026-09-28T09:00:00Z".into(),
        end_at: "2026-09-28T09:30:00Z".into(),
        color_id: Some("1".into()),
        transparent: false,
        reminders_minutes: vec![],
    };
    recorder.insert_event(&foreign).await.unwrap();
    let captured = capture(&s).await;
    assert_eq!(captured["captured"], 1);
    assert_eq!(captured["diagnostics"], json!([]));
    let raw: String =
        sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task'")
            .fetch_one(s.inner().store.pool())
            .await
            .unwrap();
    let task: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(task["category_tag"], "work");
    queries::store_calendar(
        s.inner().store.pool(),
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
    let dynamic=ok(&s,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":"Synthetic Dynamic work","tags":["work"],"category_tag":"work","duration_estimate":{"type":"fixed","seconds":600}})).await;
    generate(&s).await;
    let p = preview(&s).await;
    let dynamic_id = dynamic["task_id"].as_str().unwrap();
    assert!(p["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["task_id"] == dynamic_id)
        .unwrap()["color_id"]
        .is_null());
    ok(&s,"POST","/projection/calendar/approve",json!({"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await;
    let mut phone = recorder
        .events()
        .into_iter()
        .find(|e| e.task_id == dynamic_id)
        .unwrap();
    phone.color_id = Some("1".into());
    recorder.patch_event(&phone).await.unwrap();
    let response = capture(&s).await;
    assert_eq!(response["updated"], 1);
    assert_eq!(
        queries::get_current_state(s.inner().store.pool(), dynamic_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "completed"
    );
    assert_eq!(
        queries::get_current_state(s.inner().store.pool(), task["id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap()
            .status,
        "active"
    );
}
#[tokio::test]
async fn inverse_lists_collisions_and_every_unmapped_allowed_colour() {
    let s = state().await;
    put(&s, "work", "1").await;
    let body = list(&s).await;
    let inverse = body["inverse"].as_array().unwrap();
    assert_eq!(inverse.len(), 11);
    assert_eq!(
        inverse.iter().find(|v| v["color_id"] == "1").unwrap(),
        &json!({"color_id":"1","categories":["entertainment","work"],"status":"collision"})
    );
    assert_eq!(
        inverse.iter().find(|v| v["color_id"] == "9").unwrap(),
        &json!({"color_id":"9","categories":[],"status":"unmapped"})
    );
    println!("EVIDENCE[P1B42_test9]={}", body["inverse"]);
}
