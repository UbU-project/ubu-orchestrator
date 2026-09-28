//! Synthetic, in-process Objective and routine authoring. Nothing reaches the network.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{core::Objective, AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::planning::TimeWindowBody, build_router, config::ServerConfig, planning_time::FixedClock,
    services::routine_service::materialize, state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-09-28T09:00:00Z";
const VERSION: &str = "ubu.orchestrator.objective.v1";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn request(s: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = build_router(s.clone())
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
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
fn recurrence() -> Value {
    json!({"timezone":"America/New_York","rule":{"kind":"daily"}})
}
fn template() -> Value {
    json!({"title":"Synthetic stretch","duration_estimate":{"type":"fixed","seconds":300},
        "nominal_start":"12:00:00","placement":"static","category_tag":"health","tags":["health"]})
}
fn routine() -> Value {
    json!({"schema_version":VERSION,"title":"Synthetic stretch","mode":"evergreen",
        "recurrence":recurrence(),"routine_instance_template":template()})
}
async fn create(s: &AppState, body: Value) -> String {
    let (status, body) = request(s, "POST", "/objective", body).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["schema_version"], VERSION);
    assert_eq!(body["version"], 1);
    body["objective_id"].as_str().unwrap().into()
}
async fn edit(s: &AppState, id: &str, version: i64, mut fields: Value) -> (StatusCode, Value) {
    fields["schema_version"] = json!(VERSION);
    fields["expected_version"] = json!(version);
    request(s, "PATCH", &format!("/objective/{id}"), fields).await
}
/// The admitted row, never the response: `{version, status, payload}`.
async fn record(s: &AppState, id: &str) -> Value {
    let r = queries::get_current_state(s.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap();
    json!({"version":r.version,"status":r.status,"payload":serde_json::from_str::<Value>(&r.payload_json).unwrap()})
}
fn counters(row: &Value) -> Value {
    json!({"object_version":row["version"],
        "schedule_version":row["payload"]["recurrence"]["schedule_version"],
        "template_version":row["payload"]["routine_instance_template"]["template_version"]})
}
fn code(body: &Value, expected: &str) {
    assert_eq!(body["diagnostics"][0]["code"], expected, "{body}");
}
async fn count(s: &AppState, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(s.inner().store.pool())
        .await
        .unwrap()
}
fn record_evidence(label: &str, value: &Value) {
    println!(
        "P1B38_{label}_BEGIN\n{}\nP1B38_{label}_END",
        serde_json::to_string_pretty(value).unwrap()
    );
}
async fn import(s: &AppState, snapshot: &Value) -> Value {
    let path = std::env::temp_dir().join(format!("p1b38-{}.json", UbuId::new(ObjectType::Task)));
    std::fs::write(&path, snapshot.to_string()).unwrap();
    let (status, body) = request(
        s,
        "POST",
        "/import/quick-ubu",
        json!({"snapshot_path":path,"dry_run":false}),
    )
    .await;
    std::fs::remove_file(path).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
/// Every admitted occurrence of one routine, scrubbed of the two generated ids.
async fn occurrences(s: &AppState, objective: &str) -> Vec<Value> {
    let horizon = TimeWindowBody {
        start: UbuTimestamp::parse(NOW).unwrap().inner().unix_timestamp() as u64,
        end: UbuTimestamp::parse("2026-09-29T09:00:00Z")
            .unwrap()
            .inner()
            .unix_timestamp() as u64,
    };
    let context = materialize(s, &horizon, horizon.start).await.unwrap();
    assert!(context.diagnostics.is_empty(), "{:?}", context.diagnostics);
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id,payload_json FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=? ORDER BY json_extract(payload_json,'$.occurrence.local_date')")
        .bind(objective).fetch_all(s.inner().store.pool()).await.unwrap();
    rows.into_iter()
        .map(|(id, payload)| {
            serde_json::from_str(
                &payload
                    .replace(&id, "<task>")
                    .replace(objective, "<objective>"),
            )
            .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn a_title_alone_admits_an_active_objective_without_a_source() {
    let s = state().await;
    let id = create(
        &s,
        json!({"schema_version":VERSION,"title":"Synthetic outcome"}),
    )
    .await;
    let row = record(&s, &id).await;
    assert_eq!(row["status"], "active");
    assert_eq!(
        row["payload"],
        json!({"id":id,"title":"Synthetic outcome","status":"active",
            "provenance":{"created_at":NOW,"authority_source":"user"}})
    );
    assert!(row["payload"]["provenance"].get("source").is_none());
    let objective: Objective = serde_json::from_value(row["payload"].clone()).unwrap();
    assert_eq!(serde_json::to_value(objective).unwrap(), row["payload"]);
}

#[tokio::test]
async fn a_routine_admits_with_both_counters_at_one() {
    let s = state().await;
    // A caller-supplied counter is overwritten; both are server controlled.
    let mut body = routine();
    body["recurrence"]["schedule_version"] = json!(9);
    body["routine_instance_template"]["template_version"] = json!(9);
    let id = create(&s, body).await;
    let row = record(&s, &id).await;
    record_evidence("TEST2_ADMITTED_ROUTINE", &row["payload"]);
    let p = &row["payload"];
    assert_eq!(p["mode"], "evergreen");
    assert_eq!(p["recurrence"]["schedule_version"], 1);
    assert_eq!(p["routine_instance_template"]["template_version"], 1);
    assert_eq!(
        p["provenance"],
        json!({"created_at":NOW,"authority_source":"user"})
    );
    let objective: Objective = serde_json::from_value(p.clone()).unwrap();
    assert!(objective.recurrence.is_some() && objective.routine_instance_template.is_some());
}

#[tokio::test]
async fn a_native_routine_materializes_exactly_as_an_imported_one() {
    let native = state().await;
    let native_id = create(&native, routine()).await;
    let imported = state().await;
    let source = "00000000-0000-4000-8000-000000000001";
    let response = import(
        &imported,
        &json!({"snapshot_version":1,"task_origins":{},"store":{
        "tasks":{},"objectives":{},"bundles":{},"preferences":[],
        "routines":{source:{"id":source,"title":"Synthetic stretch","recurrence":"Daily",
            "start_time":"12:00:00","duration":[300,0],"category":"health","dynamic":false,
            "transparent":false,"reminders":[],"after":[]}}}}),
    )
    .await;
    assert_eq!(response["routines"]["created"], 1, "{response}");
    let imported_id: String = sqlx::query_scalar(
        "SELECT id FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.provenance.source.source_kind')='quick_ubu'",
    )
    .fetch_one(imported.inner().store.pool())
    .await
    .unwrap();
    let a = occurrences(&native, &native_id).await;
    let b = occurrences(&imported, &imported_id).await;
    record_evidence("TEST3_NATIVE_OCCURRENCES", &json!(a));
    record_evidence("TEST3_IMPORTED_OCCURRENCES", &json!(b));
    assert_eq!(a.len(), 1);
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(
            a["occurrence"]["key"],
            "<objective>/s1/2026-09-28T12:00:00/static/t1"
        );
        assert_eq!(a["occurrence"], b["occurrence"]);
        assert_eq!(
            a["static_window"],
            json!({"start":"2026-09-28T16:00:00Z","end":"2026-09-28T16:05:00Z"})
        );
        assert_eq!(a["static_window"], b["static_window"]);
        assert_eq!(a.get("allowed_time_range"), b.get("allowed_time_range"));
        assert_eq!(
            a["duration_estimate"],
            json!({"type":"fixed","seconds":300})
        );
        assert_eq!(a["duration_estimate"], b["duration_estimate"]);
        // Beyond the three named fields: the whole admitted payload agrees.
        assert_eq!(a, b);
    }
}

#[tokio::test]
async fn each_authoring_rejection_admits_nothing() {
    let s = state().await;
    let mut no_recurrence = routine();
    no_recurrence.as_object_mut().unwrap().remove("recurrence");
    let mut no_template = routine();
    no_template
        .as_object_mut()
        .unwrap()
        .remove("routine_instance_template");
    let mut one_time = routine();
    one_time["mode"] = json!("one_time");
    let mut default_mode = routine();
    default_mode.as_object_mut().unwrap().remove("mode");
    for (body, expected) in [
        (json!({"schema_version":VERSION}), "objective_missing_title"),
        (
            json!({"schema_version":VERSION,"title":""}),
            "objective_missing_title",
        ),
        (no_recurrence, "objective_routine_fields_incomplete"),
        (no_template, "objective_routine_fields_incomplete"),
        (one_time, "objective_routine_requires_evergreen"),
        (default_mode, "objective_routine_requires_evergreen"),
    ] {
        let (status, body) = request(&s, "POST", "/objective", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        code(&body, expected);
    }
    assert_eq!(count(&s, "objects").await, 0);
    assert_eq!(count(&s, "mutation_envelopes").await, 0);
}

#[tokio::test]
async fn each_edit_bumps_only_the_counter_it_changed() {
    let s = state().await;
    let id = create(&s, routine()).await;
    let mut evidence = json!({"created":counters(&record(&s, &id).await)});

    let mut changed = template();
    changed["nominal_start"] = json!("13:30:00");
    let (status, body) = edit(&s, &id, 1, json!({"routine_instance_template":changed})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["notice"]
            .as_str()
            .unwrap()
            .contains("next materialize"),
        "{body}"
    );
    let row = record(&s, &id).await;
    evidence["template_edit"] = counters(&row);
    assert_eq!(
        counters(&row),
        json!({"object_version":2,"schedule_version":1,"template_version":2})
    );
    assert_eq!(
        row["payload"]["routine_instance_template"]["nominal_start"],
        "13:30:00"
    );

    let weekly =
        json!({"timezone":"America/New_York","rule":{"kind":"weekly","weekdays":["tue","fri"]}});
    let (status, body) = edit(&s, &id, 2, json!({"recurrence":weekly})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("notice").is_none(), "{body}");
    let row = record(&s, &id).await;
    evidence["recurrence_edit"] = counters(&row);
    assert_eq!(
        counters(&row),
        json!({"object_version":3,"schedule_version":2,"template_version":2})
    );

    // Neither routine object changes: a rename, and both objects restated as stored.
    let stored = row["payload"].clone();
    let (status, body) = edit(
        &s,
        &id,
        3,
        json!({"title":"Synthetic stretch, renamed","recurrence":stored["recurrence"],
            "routine_instance_template":stored["routine_instance_template"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("notice").is_none(), "{body}");
    let row = record(&s, &id).await;
    evidence["edit_changing_neither"] = counters(&row);
    assert_eq!(
        counters(&row),
        json!({"object_version":4,"schedule_version":2,"template_version":2})
    );
    assert_eq!(row["payload"]["title"], "Synthetic stretch, renamed");
    record_evidence("TEST5_COUNTERS", &evidence);
}

#[tokio::test]
async fn a_stale_expected_version_conflicts_and_writes_nothing() {
    let s = state().await;
    let id = create(&s, routine()).await;
    let (status, _) = edit(&s, &id, 1, json!({"title":"Synthetic stretch, second"})).await;
    assert_eq!(status, StatusCode::OK);
    let before = record(&s, &id).await;
    let ledger = count(&s, "mutation_envelopes").await;
    let (status, body) = edit(&s, &id, 1, json!({"title":"Lost update"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    code(&body, "version_conflict");
    assert_eq!(record(&s, &id).await, before);
    assert_eq!(before["version"], 2);
    assert_eq!(count(&s, "mutation_envelopes").await, ledger);
}

#[tokio::test]
async fn a_native_objective_survives_a_later_import_untouched() {
    let s = state().await;
    let plain = create(
        &s,
        json!({"schema_version":VERSION,"title":"Synthetic routine 1","priority":40}),
    )
    .await;
    // Same title as an imported routine, at an hour the fixture leaves free.
    let mut body = routine();
    body["title"] = json!("Synthetic routine 1");
    body["routine_instance_template"]["nominal_start"] = json!("03:00:00");
    let native = create(&s, body).await;
    let before = json!({"objective":record(&s, &plain).await,"routine":record(&s, &native).await});
    let snapshot: Value =
        serde_json::from_str(include_str!("../fixtures/quick-ubu/snapshot-small.json")).unwrap();
    let later = s.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-28T09:05:00Z").unwrap(),
    ));
    let first = import(&later, &snapshot).await;
    assert_eq!(first["routines"]["created"], 5, "{first}");
    let second = import(&later, &snapshot).await;
    assert_eq!(second["routines"]["unchanged"], 5, "{second}");
    assert_eq!(second["stale"], json!([]), "{second}");
    let after = json!({"objective":record(&s, &plain).await,"routine":record(&s, &native).await});
    record_evidence("TEST7_BEFORE_IMPORT", &before);
    record_evidence("TEST7_AFTER_IMPORT", &after);
    assert_eq!(before, after);
    assert_eq!(after["routine"]["version"], 1);
    let objectives: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Objective'")
            .fetch_one(s.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(objectives, 7);
}

#[tokio::test]
async fn reads_return_the_authored_routine_with_its_template() {
    let s = state().await;
    let plain = create(
        &s,
        json!({"schema_version":VERSION,"title":"Synthetic outcome","priority":40}),
    )
    .await;
    let id = create(&s, routine()).await;
    let (status, list) = request(&s, "GET", "/objectives", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["schema_version"], VERSION);
    let mut expected = vec![
        json!({"objective_id":plain,"title":"Synthetic outcome","status":"active","priority":40,
            "mode":"one_time","is_routine":false,"version":1}),
        json!({"objective_id":id,"title":"Synthetic stretch","status":"active",
            "mode":"evergreen","is_routine":true,"version":1}),
    ];
    // Both share a creation instant under the fixed clock, so id decides.
    expected.sort_by_key(|o| o["objective_id"].as_str().unwrap().to_owned());
    assert_eq!(list["objectives"], json!(expected));
    let (status, body) = request(&s, "GET", &format!("/objective/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 1);
    assert_eq!(body["is_routine"], true);
    assert_eq!(body["payload"], record(&s, &id).await["payload"]);
    assert_eq!(
        body["payload"]["recurrence"],
        json!({"timezone":"America/New_York","rule":{"kind":"daily"},"schedule_version":1})
    );
    let mut stored = template();
    stored["template_version"] = json!(1);
    assert_eq!(body["payload"]["routine_instance_template"], stored);
    let missing = UbuId::new(ObjectType::Objective);
    let (status, body) = request(&s, "GET", &format!("/objective/{missing}"), Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    code(&body, "unknown_objective");
}

fn routine_at(title: &str, start: &str) -> Value {
    let mut body = routine();
    body["title"] = json!(title);
    body["routine_instance_template"]["title"] = json!(title);
    body["routine_instance_template"]["nominal_start"] = json!(start);
    body
}
fn quick_routine(n: u8, title: &str, start: &str) -> Value {
    json!({"id":format!("00000000-0000-4000-8000-{n:012}"),"title":title,"recurrence":"Daily",
        "start_time":start,"duration":[300,0],"dynamic":false,"transparent":false,"reminders":[],"after":[]})
}
fn quick_snapshot(routines: Vec<Value>) -> Value {
    let routines: serde_json::Map<String, Value> = routines
        .into_iter()
        .map(|r| (r["id"].as_str().unwrap().to_owned(), r))
        .collect();
    json!({"snapshot_version":1,"task_origins":{},"store":{"routines":routines,"tasks":{},
        "objectives":{},"bundles":{},"preferences":[]}})
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/quick-ubu/snapshot-small.json")).unwrap()
}
async fn try_import(s: &AppState, snapshot: &Value) -> (StatusCode, Value) {
    let path = std::env::temp_dir().join(format!("p1b39-{}.json", UbuId::new(ObjectType::Task)));
    std::fs::write(&path, snapshot.to_string()).unwrap();
    let result = request(
        s,
        "POST",
        "/import/quick-ubu",
        json!({"snapshot_path":path,"dry_run":false}),
    )
    .await;
    std::fs::remove_file(path).unwrap();
    result
}
/// Admission at the store boundary, as any writer that predates or sidesteps
/// `objective_authoring` would do it. Nothing here consults the overlap check.
async fn admit_unchecked(s: &AppState, title: &str, start: &str) -> String {
    let id = UbuId::new(ObjectType::Objective);
    let now = UbuTimestamp::parse(NOW).unwrap();
    let mut payload = routine_at(title, start);
    let object = payload.as_object_mut().unwrap();
    object.remove("schema_version");
    object.insert("id".into(), json!(id));
    object.insert("status".into(), json!("active"));
    object.insert(
        "provenance".into(),
        json!({"created_at":NOW,"authority_source":"user"}),
    );
    payload["recurrence"]["schedule_version"] = json!(1);
    payload["routine_instance_template"]["template_version"] = json!(1);
    let envelope = s
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        s.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: "Objective".into(),
            version: 1,
            status: "active".into(),
            compartment_label: "user-capture".into(),
            payload,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap();
    id.to_string()
}
fn overlap_evidence(label: &str, value: &Value) {
    println!(
        "P1B39_{label}_BEGIN\n{}\nP1B39_{label}_END",
        serde_json::to_string_pretty(value).unwrap()
    );
}
fn overlap_codes(body: &Value) {
    let diagnostics = body["diagnostics"].as_array().unwrap();
    assert!(!diagnostics.is_empty(), "{body}");
    assert!(
        diagnostics
            .iter()
            .all(|d| d["code"] == "objective_routine_overlap"),
        "{body}"
    );
}

#[tokio::test]
async fn an_overlapping_native_routine_is_rejected_and_the_import_still_succeeds() {
    let s = state().await;
    let first = create(&s, routine_at("Synthetic stretch", "03:00:00")).await;
    let (objects, ledger) = (
        count(&s, "objects").await,
        count(&s, "mutation_envelopes").await,
    );
    let (status, body) = request(
        &s,
        "POST",
        "/objective",
        routine_at("Synthetic walk", "03:02:00"),
    )
    .await;
    overlap_evidence("TEST1_REJECTION", &body);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    overlap_codes(&body);
    assert_eq!(body["diagnostics"].as_array().unwrap().len(), 1);
    let message = body["diagnostics"][0]["message"].as_str().unwrap();
    for expected in [
        "Conflict with another routine",
        "(Synthetic walk) 03:02:00-03:07:00",
        &format!("`{first}` (Synthetic stretch) 03:00:00-03:05:00"),
        "first on 2026-09-29",
    ] {
        assert!(message.contains(expected), "{message}");
    }
    assert_eq!(count(&s, "objects").await, objects);
    assert_eq!(count(&s, "mutation_envelopes").await, ledger);
    assert_eq!(objects, 1);

    let (status, imported) = try_import(&s, &fixture()).await;
    overlap_evidence(
        "TEST1_IMPORT",
        &json!({"status":status.as_u16(),"body":imported}),
    );
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["routines"]["created"], 5, "{imported}");
    let routines: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type='Objective'")
            .fetch_one(s.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(routines, 6);
}

#[tokio::test]
async fn overlapping_routines_admitted_past_the_check_still_lock_the_importer() {
    let s = state().await;
    admit_unchecked(&s, "Synthetic stretch", "03:00:00").await;
    admit_unchecked(&s, "Synthetic walk", "03:02:00").await;
    let objects = count(&s, "objects").await;
    assert_eq!(objects, 2);
    let (status, body) = try_import(&s, &fixture()).await;
    overlap_evidence(
        "TEST2_LOCKOUT",
        &json!({"status":status.as_u16(),"body":body}),
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["diagnostics"][0]["code"], "overlapping_routines");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("nothing was imported"),
        "{body}"
    );
    assert_eq!(count(&s, "objects").await, objects);
}

#[tokio::test]
async fn a_native_routine_overlapping_an_imported_one_is_rejected_naming_both() {
    let s = state().await;
    let (status, imported) = try_import(
        &s,
        &quick_snapshot(vec![quick_routine(1, "Imported stretch", "12:00:00")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    let imported_id: String =
        sqlx::query_scalar("SELECT id FROM objects WHERE object_type='Objective'")
            .fetch_one(s.inner().store.pool())
            .await
            .unwrap();
    let objects = count(&s, "objects").await;
    let (status, body) = request(
        &s,
        "POST",
        "/objective",
        routine_at("Synthetic walk", "12:02:00"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    overlap_codes(&body);
    let message = body["diagnostics"][0]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "`{imported_id}` (Imported stretch) 12:00:00-12:05:00"
        )),
        "{message}"
    );
    assert!(
        message.contains("(Synthetic walk) 12:02:00-12:07:00"),
        "{message}"
    );
    assert_eq!(count(&s, "objects").await, objects);
    // Clear of the imported routine, the same routine is admitted.
    create(&s, routine_at("Synthetic walk", "12:05:00")).await;
}

#[tokio::test]
async fn an_edit_that_moves_a_routine_onto_another_is_rejected_unchanged() {
    let s = state().await;
    let first = create(&s, routine_at("Synthetic stretch", "03:00:00")).await;
    let second = create(&s, routine_at("Synthetic walk", "04:00:00")).await;
    let before = record(&s, &second).await;
    let ledger = count(&s, "mutation_envelopes").await;
    let mut moved = template();
    moved["title"] = json!("Synthetic walk");
    moved["nominal_start"] = json!("03:02:00");
    let (status, body) = edit(&s, &second, 1, json!({"routine_instance_template":moved})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    overlap_codes(&body);
    let message = body["diagnostics"][0]["message"].as_str().unwrap();
    for expected in [
        format!("`{second}` (Synthetic walk) 03:02:00-03:07:00"),
        format!("`{first}` (Synthetic stretch) 03:00:00-03:05:00"),
    ] {
        assert!(message.contains(&expected), "{message}");
    }
    assert_eq!(record(&s, &second).await, before);
    assert_eq!(before["version"], 1);
    assert_eq!(count(&s, "mutation_envelopes").await, ledger);
    // Withdrawing a routine is never an overlap, whatever its template says.
    let (status, body) = edit(
        &s,
        &second,
        1,
        json!({"status":"abandoned","routine_instance_template":moved}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_edit_onto_the_routines_own_previous_window_succeeds() {
    let s = state().await;
    let id = create(&s, routine_at("Synthetic stretch", "03:00:00")).await;
    let mut moved = template();
    moved["nominal_start"] = json!("03:02:00");
    let (status, body) = edit(&s, &id, 1, json!({"routine_instance_template":moved})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = record(&s, &id).await;
    assert_eq!(row["version"], 2);
    assert_eq!(
        row["payload"]["routine_instance_template"]["nominal_start"],
        "03:02:00"
    );
    // A rename of an unchanged routine is compared with nothing but itself.
    let (status, body) = edit(&s, &id, 2, json!({"title":"Synthetic stretch, renamed"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A routine longer than its own period collides with itself, on either path.
    let mut marathon = template();
    marathon["duration_estimate"] = json!({"type":"fixed","seconds":36 * 3600});
    let (status, body) = edit(&s, &id, 3, json!({"routine_instance_template":marathon})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    overlap_codes(&body);
    assert!(
        body["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("Conflict with itself"),
        "{body}"
    );
    assert_eq!(record(&s, &id).await["version"], 3);
}

#[tokio::test]
async fn the_import_rejection_says_which_routines_are_native() {
    const ADVICE: &str = "1 overlapping routine pair in 1 group; nothing was imported. Routines must not overlap: stagger their start times, shorten one, or make one transparent.";
    let native = state().await;
    let id = create(&native, routine_at("Synthetic stretch", "12:00:00")).await;
    let (status, with_native) = try_import(
        &native,
        &quick_snapshot(vec![quick_routine(1, "Imported walk", "12:02:00")]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{with_native}");
    let imported = state().await;
    let (status, imported_only) = try_import(
        &imported,
        &quick_snapshot(vec![
            quick_routine(1, "Imported walk", "12:02:00"),
            quick_routine(2, "Imported stretch", "12:00:00"),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{imported_only}");
    overlap_evidence("TEST6_NATIVE", &with_native);
    overlap_evidence("TEST6_IMPORTED", &imported_only);

    assert_eq!(
        with_native["error"],
        format!("{ADVICE} Routines marked natively authored are not in routine.json: edit them through PATCH /objective/:objective_id.")
    );
    assert_eq!(
        with_native["diagnostics"][0]["code"],
        "overlapping_routines"
    );
    let message = with_native["diagnostics"][0]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "`{id}` (Synthetic stretch; natively authored) 12:00:00-12:05:00"
        )),
        "{message}"
    );
    assert!(
        message
            .contains("`00000000-0000-4000-8000-000000000001` (Imported walk) 12:02:00-12:07:00"),
        "{message}"
    );

    assert_eq!(imported_only["error"], ADVICE);
    assert_eq!(
        imported_only["diagnostics"][0]["message"],
        "Routines overlap each other, first 2026-09-28: `00000000-0000-4000-8000-000000000002` (Imported stretch) 12:00:00-12:05:00; `00000000-0000-4000-8000-000000000001` (Imported walk) 12:02:00-12:07:00 (1 pair, up to 366 dates in the next year)"
    );
    assert_eq!(count(&native, "objects").await, 1);
    assert_eq!(count(&imported, "objects").await, 0);
}
