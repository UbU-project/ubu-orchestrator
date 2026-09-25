use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::task::TASK_CAPTURE_SCHEMA_VERSION, build_router, config::ServerConfig, errors::AppError,
    planning_time::FixedClock, state::AppState,
};
use ubu_store::{
    models::{
        calendar_record::NewCalendarRecord,
        object_record::{NewObjectRecord, ObjectRecord},
    },
    queries,
};
const NOW: &str = "2026-09-24T09:00:00Z";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
async fn capture(state: &AppState, mut fields: Value) -> Value {
    fields["schema_version"] = json!(TASK_CAPTURE_SCHEMA_VERSION);
    let (status, body) = request(state, "POST", "/task", fields).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["version"], 1);
    assert_eq!(body["schema_version"], TASK_CAPTURE_SCHEMA_VERSION);
    body
}
async fn edit(state: &AppState, id: &str, version: i64, mut fields: Value) -> (StatusCode, Value) {
    fields["schema_version"] = json!(TASK_CAPTURE_SCHEMA_VERSION);
    fields["expected_version"] = json!(version);
    request(state, "PATCH", &format!("/task/{id}"), fields).await
}
async fn record(state: &AppState, id: &str) -> ObjectRecord {
    queries::get_current_state(state.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap()
}
fn payload(record: &ObjectRecord) -> Value {
    serde_json::from_str(&record.payload_json).unwrap()
}
fn diagnostic(body: &Value, code: &str) {
    assert_eq!(body["diagnostics"][0]["code"], code, "{body}");
}
async fn generate(state: &AppState) -> Value {
    let end = "2026-09-24T23:59:00Z";
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: NOW.into(),
            window_end: end.into(),
            payload: json!({"windows":[{"start":NOW,"end":end}]}),
            created_at: NOW.into(),
        },
    )
    .await
    .unwrap();
    let (status, body) = request(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":end}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
struct SnapshotFile(std::path::PathBuf);
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn snapshot(routines: Value) -> Value {
    json!({"snapshot_version":1,"store":{"routines":routines,"tasks":{},"objectives":{},"bundles":{},"preferences":[]},"task_origins":{}})
}
fn uid() -> &'static str {
    "00000000-0000-4000-8000-000000000001"
}
fn routine_snapshot() -> Value {
    snapshot(
        json!({uid():{"id":uid(),"title":"Synthetic routine","recurrence":"Daily","start_time":"10:00:00","duration":[300,0],"dynamic":true,"latest_tod":"11:00:00"}}),
    )
}
async fn import(state: &AppState, snapshot: Value) -> Value {
    let file = SnapshotFile(
        std::env::temp_dir().join(format!("p1b27-{}.json", UbuId::new(ObjectType::Task))),
    );
    std::fs::write(&file.0, snapshot.to_string()).unwrap();
    let (status, body) = request(
        state,
        "POST",
        "/import/quick-ubu",
        json!({"snapshot_path":file.0,"timezone":"UTC"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn capture_is_planned_and_records_user_provenance_without_a_source() {
    let state = state().await;
    let captured = capture(
        &state,
        json!({"title":"Call the plumber","duration_estimate":{"type":"fixed","seconds":900}}),
    )
    .await;
    let id = captured["task_id"].as_str().unwrap();
    let stored = record(&state, id).await;
    let value = payload(&stored);
    assert_eq!(stored.compartment_label, "user-capture");
    assert_eq!(stored.status, "active");
    assert_eq!(
        value,
        json!({"id":id,"title":"Call the plumber","status":"active","duration_estimate":{"type":"fixed","seconds":900},"provenance":{"created_at":NOW,"authority_source":"user"}})
    );
    assert!(value["provenance"].get("source").is_none());
    println!("P1B27_CAPTURED {}", value);
    let generated = generate(&state).await;
    assert_eq!(generated["status"], "ok", "{generated}");
    let steps = generated["plan"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["task_id"], id);
    let title =
        payload(&record(&state, steps[0]["task_id"].as_str().unwrap()).await)["title"].clone();
    assert_eq!(title, "Call the plumber");
}

#[tokio::test]
async fn title_only_capture_works_and_server_owned_fields_are_rejected() {
    let state = state().await;
    capture(&state, json!({"title":"Minimal Task"})).await;
    for mut fields in [json!({}), json!({"title":""}), json!({"title":null})] {
        fields["schema_version"] = json!(TASK_CAPTURE_SCHEMA_VERSION);
        let (status, body) = request(&state, "POST", "/task", fields).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        diagnostic(&body, "missing_title");
    }
    for key in [
        "occurrence",
        "status",
        "id",
        "provenance",
        "moot_reason_code",
        "assignee",
        "unknown",
    ] {
        let mut fields =
            json!({"schema_version":TASK_CAPTURE_SCHEMA_VERSION,"title":"Do not admit"});
        fields[key] = json!("forged");
        let (status, body) = request(&state, "POST", "/task", fields).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        diagnostic(&body, "unsupported_capture_field");
        assert!(body["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains(key));
    }
    for (fields, code) in [
        (json!({"title":"No schema"}), "missing_schema_version"),
        (
            json!({"title":"Wrong schema","schema_version":"wrong"}),
            "unknown_schema_version",
        ),
    ] {
        let (status, body) = request(&state, "POST", "/task", fields).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        diagnostic(&body, code);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Task'")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn edit_updates_at_the_expected_version_and_preserves_server_metadata() {
    let state = state().await;
    let captured = capture(&state, json!({"title":"Original"})).await;
    let id = captured["task_id"].as_str().unwrap();
    let before = record(&state, id).await;
    let later = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-24T09:05:00Z").unwrap(),
    ));
    let (status,body)=edit(&later,id,1,json!({"title":"Updated","description":"Synthetic detail","duration_estimate":{"type":"fixed","seconds":600},"tags":["home"],"category_tag":"home"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 2);
    let after = record(&state, id).await;
    assert_eq!(after.version, 2);
    assert_eq!(payload(&after)["title"], "Updated");
    assert_eq!(after.status, before.status);
    assert_eq!(after.compartment_label, before.compartment_label);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(
        payload(&after)["provenance"],
        payload(&before)["provenance"]
    );
    for key in ["status", "provenance", "id", "occurrence"] {
        let mut fields = json!({});
        fields[key] = Value::Null;
        let (status, body) = edit(&state, id, 2, fields).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        diagnostic(&body, "unsupported_capture_field");
    }
    let (status, body) = edit(
        &state,
        &UbuId::new(ObjectType::Task).to_string(),
        1,
        json!({"title":"Missing"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    diagnostic(&body, "unknown_task");
    let (status, body) = request(
        &state,
        "PATCH",
        &format!("/task/{id}"),
        json!({"schema_version":TASK_CAPTURE_SCHEMA_VERSION,"title":"No version"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(record(&state, id).await.payload_json, after.payload_json);
}

#[tokio::test]
async fn stale_edits_and_store_preconditions_conflict_without_writing() {
    let state = state().await;
    let captured = capture(&state, json!({"title":"Original"})).await;
    let id = captured["task_id"].as_str().unwrap();
    assert_eq!(
        edit(&state, id, 1, json!({"title":"Kept"})).await.0,
        StatusCode::OK
    );
    let before = record(&state, id).await;
    let (status, body) = edit(&state, id, 1, json!({"title":"Must not write"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    diagnostic(&body, "version_conflict");
    let message = body["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains("expected version 1") && message.contains("current version 2"));
    println!("P1B27_CONFLICT_409 {}", body);
    let after = record(&state, id).await;
    assert_eq!(after.payload_json, before.payload_json);
    assert_eq!(after.version, before.version);
    assert_eq!(after.updated_at, before.updated_at);
    // Exercise the real store guard deterministically, as if a write landed
    // after the service's advisory read and before its admission.
    let envelope = state
        .envelope_for(
            [(UbuId::parse(id).unwrap(), VersionRef::Version(1))]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            state.planning_now(),
        )
        .unwrap();
    let mut stale = payload(&before);
    stale["title"] = json!("Lost race");
    let error = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.into(),
            object_type: "Task".into(),
            version: 2,
            status: before.status.clone(),
            compartment_label: before.compartment_label.clone(),
            payload: stale,
            created_at: before.created_at.clone(),
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ubu_store::StoreError::PreconditionFailed { .. }
    ));
    assert_eq!(
        AppError::Store(error).into_response().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(record(&state, id).await.payload_json, before.payload_json);
    for error in [
        ubu_store::StoreError::MissingTargetPrecondition {
            object_id: id.into(),
        },
        ubu_store::StoreError::DuplicateOccurrenceKey {
            key: "synthetic".into(),
        },
    ] {
        assert_eq!(
            AppError::Store(error).into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}

#[tokio::test]
async fn null_clears_optional_fields_and_invalid_tasks_are_not_admitted() {
    let state = state().await;
    let captured = capture(
        &state,
        json!({"title":"Clear due date","due_at":"2026-09-25T09:00:00Z"}),
    )
    .await;
    let id = captured["task_id"].as_str().unwrap();
    let (status, body) = edit(&state, id, 1, json!({"due_at":null})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let before = record(&state, id).await;
    assert!(payload(&before).get("due_at").is_none());
    let invalid = json!({"duration_estimate":{"type":"fixed","seconds":0}});
    let mut task: ubu_core::core::Task = serde_json::from_value(payload(&before)).unwrap();
    task.duration_estimate = Some(ubu_core::core::TaskDurationEstimate::Fixed { seconds: 0 });
    let expected = task.validate().unwrap_err().to_string();
    let (status, body) = edit(&state, id, 2, invalid).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains(&expected),
        "{body}"
    );
    assert_eq!(record(&state, id).await.payload_json, before.payload_json);
    assert_eq!(record(&state, id).await.version, 2);
    let (status, body) = edit(&state, id, 2, json!({"title":null})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    diagnostic(&body, "missing_title");
    assert_eq!(record(&state, id).await.payload_json, before.payload_json);
}

#[tokio::test]
async fn materialized_routine_occurrences_reject_edits_before_version_checks() {
    let state = state().await;
    import(&state, routine_snapshot()).await;
    generate(&state).await;
    let row:ObjectRecord=sqlx::query_as("SELECT * FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence') IS NOT NULL")
        .fetch_one(state.inner().store.pool()).await.unwrap();
    let routine = payload(&row)["occurrence"]["routine_objective_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for version in [row.version, row.version + 10] {
        let (status, body) = edit(&state, &row.id, version, json!({"title":"Vanishes"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        diagnostic(&body, "routine_occurrence_not_editable");
        assert!(body["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains(&routine));
        assert_eq!(record(&state, &row.id).await.payload_json, row.payload_json);
        if version == row.version {
            println!("P1B27_OCCURRENCE_REJECTION {}", body);
        }
    }
}

#[tokio::test]
async fn capture_survives_two_import_rounds_without_stale_or_diverged_labels() {
    let state = state().await;
    let captured = capture(&state, json!({"title":"Call the plumber"})).await;
    let id = captured["task_id"].as_str().unwrap();
    let before = record(&state, id).await;
    let mut fixture = routine_snapshot();
    let task_id = "00000000-0000-4000-8000-000000000002";
    fixture["store"]["tasks"][task_id] = json!({"id":task_id,"title":"Synthetic imported Task","status":"Backlog","est_duration":[300,0]});
    fixture["task_origins"][task_id] = json!("manual");
    for (round, expected) in [
        (1, json!({"created":1,"updated":0,"unchanged":0})),
        (2, json!({"created":0,"updated":1,"unchanged":0})),
    ] {
        if round == 2 {
            fixture["store"]["tasks"][task_id]["title"] = json!("Changed imported Task");
        }
        let imported = import(&state, fixture.clone()).await;
        assert_eq!(imported["tasks"], expected);
        assert_eq!(
            imported["routines"],
            if round == 1 {
                json!({"created":1,"updated":0,"unchanged":0})
            } else {
                json!({"created":0,"updated":0,"unchanged":1})
            }
        );
        for list in ["stale", "diverged"] {
            assert!(imported[list]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["id"] != id));
        }
        let after = record(&state, id).await;
        assert_eq!(after.status, "active");
        assert_eq!(payload(&after)["title"], "Call the plumber");
        assert_eq!(after.payload_json, before.payload_json);
        assert_eq!(after.version, 1);
        println!(
            "P1B27_IMPORT_ROUND_{round} {}",
            json!({"routines":imported["routines"],"tasks":imported["tasks"],"preferences":imported["preferences"],"stale":imported["stale"],"diverged":imported["diverged"]})
        );
    }
}
