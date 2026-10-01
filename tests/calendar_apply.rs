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
    config::ServerConfig,
    planning_time::FixedClock,
    services::{
        calendar_apply,
        calendar_client::{RecordedCalendarCall, RecordingCalendarApi},
    },
    state::AppState,
};
use ubu_store::{
    models::{calendar_record::NewCalendarRecord, projection_record::NewProjectionResultRecord},
    queries,
};
const NOW: &str = "2026-09-25T08:00:00Z";
const END: &str = "2026-09-25T20:00:00Z";
const PREVIEW: &str = "/projection/calendar/preview";
const APPROVE: &str = "/projection/calendar/approve";
const SCHEMA: &str = "ubu.orchestrator.calendar_projection_approval.v1";
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
async fn ok(state: &AppState, method: &str, uri: &str, body: Value) -> Value {
    let (status, body) = request(state, method, uri, body).await;
    assert!(status.is_success(), "{uri}: {status} {body}");
    body
}
/// A Task that did not happen, written as the orchestrator writes one.
async fn make_moot(state: &AppState, id: &str) {
    let row = queries::get_current_state(state.inner().store.pool(), id).await.unwrap().unwrap();
    let mut payload: Value = serde_json::from_str(&row.payload_json).unwrap();
    payload["status"] = "moot".into();
    payload["moot_reason_code"] = "user_declared_moot".into();
    let now = state.planning_now();
    let envelope = state
        .envelope_for(
            [(UbuId::parse(id).unwrap(), ubu_core::VersionRef::Version(row.version as u64))].into_iter().collect(),
            ubu_core::AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        ubu_store::models::object_record::NewObjectRecord {
            id: id.into(),
            object_type: ObjectType::Task.as_str().into(),
            version: row.version + 1,
            status: "moot".into(),
            compartment_label: row.compartment_label.clone(),
            payload,
            created_at: row.created_at.clone(),
            updated_at: now.to_string(),
        },
    )
    .await
    .unwrap();
}
async fn generate(state: &AppState) {
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: NOW.into(),
            window_end: END.into(),
            payload: json!({"windows":[{"start":NOW,"end":END}]}),
            created_at: state.planning_now().to_string(),
        },
    )
    .await
    .unwrap();
    let body = ok(
        state,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":END}}),
    )
    .await;
    assert!(body["plan"].is_object(), "{body}");
}
async fn setup() -> (AppState, Arc<RecordingCalendarApi>) {
    let client = Arc::new(RecordingCalendarApi::new());
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
        .with_calendar_api(client.clone());
    for (title, start, end, category, capacity) in [
        ("Breakfast", "09:00", "09:15", "personal", true),
        ("Work Time", "10:00", "10:15", "work", false),
        ("Standup", "11:00", "11:15", "business", true),
    ] {
        ok(&state,"POST","/task",json!({"schema_version":"ubu.orchestrator.task_capture.v1","title":title,
            "static_window":{"start":format!("2026-09-25T{start}:00Z"),"end":format!("2026-09-25T{end}:00Z")},
            "category_tag":category,"tags":[category],"occupies_capacity":capacity})).await;
    }
    generate(&state).await;
    (state, client)
}
async fn preview(state: &AppState) -> Value {
    ok(state, "GET", PREVIEW, Value::Null).await
}
async fn approve(state: &AppState, preview: &Value, authority: &str) -> Value {
    ok(state,"POST",APPROVE,json!({"schema_version":SCHEMA,"preview_id":preview["preview_id"],"authority_source":authority,"export_mode":"mock"})).await
}
async fn results(state: &AppState) -> Vec<Value> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT payload_json FROM projection_results ORDER BY rowid")
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).unwrap())
        .collect()
}
async fn boundaries(state: &AppState) -> Vec<Value> {
    let rows:Vec<String>=sqlx::query_scalar("SELECT payload_json FROM logs WHERE event_type='compartment_boundary_decided' ORDER BY rowid").fetch_all(state.inner().store.pool()).await.unwrap();
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).unwrap())
        .collect()
}
fn expected_inserts(preview: &Value) -> Value {
    Value::Array(
        preview["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| json!({"kind":"insert_event","event":op["event"]}))
            .collect(),
    )
}

#[tokio::test]
async fn mock_apply_records_one_permitted_insert_per_event_in_order() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    assert!(client.recorded_calls().is_empty());
    assert_eq!(p["events"].as_array().unwrap().len(), 3);
    let applied = approve(&state, &p, "automation_worker").await;
    assert_eq!(applied["status"], "applied", "{applied}");
    let calls = serde_json::to_value(client.recorded_calls()).unwrap();
    assert_eq!(calls, expected_inserts(&p));
    println!("P1B29_CALLS_1 {calls}");
    assert!(applied["operation_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["status"] == "applied"));
    assert_eq!(
        applied["applied_events"],
        serde_json::to_value(client.events()).unwrap()
    );
    let stored = results(&state).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["applied_events"], applied["applied_events"]);
    let stored_outcomes: Vec<ubu_core::projection::OperationResult> =
        serde_json::from_value(stored[0]["operation_results"].clone()).unwrap();
    let response_outcomes: Vec<ubu_core::projection::OperationResult> =
        serde_json::from_value(applied["operation_results"].clone()).unwrap();
    assert_eq!(stored_outcomes, response_outcomes);
    let logs = boundaries(&state).await;
    assert_eq!(logs.len(), 3);
    assert!(logs
        .iter()
        .all(|l| l["adjudication_result"] == "accepted"
            && l["authority_source"] == "automation_worker"));
    let stored = calendar_apply::load_preview(
        state.inner().store.pool(),
        p["preview_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    for operation in &stored.operations {
        let lowered = calendar_apply::lower_operation(operation).unwrap();
        assert_eq!(
            lowered.kind,
            ubu_core::projection::ProjectionOperationKind::Create
        );
        assert_eq!(lowered.target.source_kind, "google_calendar");
        assert_eq!(
            lowered.payload.as_ref().unwrap()["external_id"],
            lowered.target.source_id
        );
    }
}

#[tokio::test]
async fn next_preview_after_apply_has_zero_operations_and_ignores_github_results() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    approve(&state, &p, "automation_worker").await;
    // A newer result for another surface is not the Calendar's applied set.
    queries::store_projection_result(state.inner().store.pool(),NewProjectionResultRecord {
        id:UbuId::new(ObjectType::Snapshot).to_string(),preview_id:p["preview_id"].as_str().unwrap().into(),status:"applied".into(),
        payload:json!({"schema_version":"ubu.orchestrator.projection_result.v1","result":{"synthetic_github_result":true}}),created_at:NOW.into(),
    }).await.unwrap();
    let second = preview(&state).await;
    assert_eq!(second["operations"], json!([]));
    println!(
        "P1B29_SECOND_PREVIEW operations={}",
        second["operations"].as_array().unwrap().len()
    );
    assert_eq!(second["events"], p["events"]);
    assert_eq!(client.recorded_calls().len(), 3);
    // An already superseded preview cannot replay its inserts against a new base.
    let (status,body)=request(&state,"POST",APPROVE,json!({"schema_version":SCHEMA,"preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["diagnostics"][0]["code"],
        "calendar_projection_conflict"
    );
    assert_eq!(client.recorded_calls().len(), 3);
    let unchanged = approve(&state, &second, "automation_worker").await;
    assert_eq!(unchanged["status"], "applied");
    assert_eq!(unchanged["operation_results"], json!([]));
    assert_eq!(client.recorded_calls().len(), 3);
}

#[tokio::test]
async fn changed_day_patches_one_event_and_deletes_one_event() {
    let (state, client) = setup().await;
    let first = preview(&state).await;
    approve(&state, &first, "automation_worker").await;
    let events = first["events"].as_array().unwrap();
    let breakfast = events.iter().find(|e| e["summary"] == "Breakfast").unwrap();
    let standup = events.iter().find(|e| e["summary"] == "Standup").unwrap();
    let later = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-25T08:05:00Z").unwrap(),
    ));
    ok(&later,"PATCH",&format!("/task/{}",breakfast["task_id"].as_str().unwrap()),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"static_window":{"start":"2026-09-25T09:30:00Z","end":"2026-09-25T09:45:00Z"}})).await;
    // Standup did not happen: it is moot, so it leaves the Plan and its event is
    // deleted. Until P1B-53 this test completed it instead; a completed Task's
    // event is now frozen as the record of what was done, and takes no operation.
    make_moot(&later, standup["task_id"].as_str().unwrap()).await;
    generate(&later).await;
    let changed = preview(&later).await;
    let ops = changed["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 2, "{changed}");
    assert_eq!(ops[0]["kind"], "update");
    assert_eq!(ops[1]["kind"], "delete");
    client.clear_recorded_calls();
    let applied = approve(&later, &changed, "automation_worker").await;
    assert_eq!(applied["status"], "applied");
    let calls = serde_json::to_value(client.recorded_calls()).unwrap();
    assert_eq!(
        calls,
        json!([{"kind":"patch_event","event":ops[0]["event"]},{"kind":"delete_event","external_id":standup["external_id"]}])
    );
    println!("P1B29_CALLS_3 {calls}");
    assert_eq!(client.events().len(), 2);
    assert_eq!(preview(&later).await["operations"], json!([]));
    let stored = calendar_apply::load_preview(
        state.inner().store.pool(),
        changed["preview_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    let lowered = calendar_apply::lower_operation(&stored.operations[1]).unwrap();
    assert_eq!(
        lowered.kind,
        ubu_core::projection::ProjectionOperationKind::Delete
    );
    assert_eq!(
        lowered.payload.unwrap(),
        json!({"external_id":standup["external_id"],"summary":"Standup"})
    );
}

/// P1B-29 required a `user` approve to be rejected here. P1B-43 reversed that:
/// the request's authority names the approver and is not what the gate sees.
#[tokio::test]
async fn every_approver_authority_exports_as_the_automation_worker() {
    for approver in [
        "user",
        "user_override",
        "delegated",
        "automation_worker",
        "policy",
        "system",
    ] {
        let (state, client) = setup().await;
        let p = preview(&state).await;
        let applied = approve(&state, &p, approver).await;
        assert_eq!(applied["status"], "applied", "{approver}: {applied}");
        assert_eq!(client.recorded_calls().len(), 3, "{approver}");
        assert_eq!(applied["diagnostics"], json!([]), "{approver}");
        assert_eq!(preview(&state).await["operations"], json!([]), "{approver}");
    }
}

#[tokio::test]
async fn no_external_export_policy_is_rejected_without_client_calls_and_logged() {
    let (state, client) = setup().await;
    let p = ok(
        &state,
        "GET",
        "/projection/calendar/preview?no_external_export=true",
        Value::Null,
    )
    .await;
    let applied = approve(&state, &p, "automation_worker").await;
    assert_eq!(applied["status"], "failed");
    assert!(client.recorded_calls().is_empty());
    let logs = boundaries(&state).await;
    assert_eq!(logs.len(), 3);
    assert!(logs.iter().all(|l| l["adjudication_result"] == "rejected"
        && l["reason"].as_str().unwrap().contains("no_external_export")));
    let reasons: Vec<_> = logs.iter().map(|l| l["reason"].clone()).collect();
    println!("P1B29_REJECT_POLICY {}", json!(reasons));
    assert!(applied["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["code"] == "calendar_export_rejected"));
    assert_eq!(results(&state).await[0]["applied_events"], json!([]));
}

#[tokio::test]
async fn live_mode_is_refused_without_calls_or_delivery_records() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    let (status,body)=request(&state,"POST",APPROVE,json!({"schema_version":SCHEMA,"preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"live"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["diagnostics"][0]["code"],
        "calendar_live_export_unconfigured"
    );
    assert!(client.recorded_calls().is_empty());
    assert!(results(&state).await.is_empty());
    assert!(boundaries(&state).await.is_empty());
    for (version, code) in [
        (Value::Null, "missing_schema_version"),
        (json!("unknown"), "unknown_schema_version"),
    ] {
        let (status,body)=request(&state,"POST",APPROVE,json!({"schema_version":version,"preview_id":p["preview_id"],"authority_source":"automation_worker","export_mode":"mock"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["diagnostics"][0]["code"], code);
    }
    assert!(client.recorded_calls().is_empty());
}

#[tokio::test]
async fn partial_apply_persists_only_landed_events_and_reproposes_the_failure() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    let failed = p["operations"][0]["event"]["external_id"].as_str().unwrap();
    client.fail_for(failed);
    let applied = approve(&state, &p, "automation_worker").await;
    assert_eq!(applied["status"], "partial");
    assert_eq!(client.recorded_calls().len(), 3);
    assert_eq!(client.events().len(), 2);
    let persisted = results(&state).await;
    assert_eq!(persisted[0]["status"], "partial");
    assert_eq!(
        persisted[0]["applied_events"],
        serde_json::to_value(client.events()).unwrap()
    );
    let remaining = preview(&state).await;
    assert_eq!(remaining["operations"].as_array().unwrap().len(), 1);
    assert_eq!(remaining["operations"][0]["event"]["external_id"], failed);
    // An entirely failed follow-up must not hide the earlier partial success.
    let failed_retry = approve(&state, &remaining, "automation_worker").await;
    assert_eq!(failed_retry["status"], "failed");
    assert_eq!(client.recorded_calls().len(), 4);
    assert_eq!(
        calendar_apply::last_applied_events(state.inner().store.pool())
            .await
            .unwrap(),
        client.events()
    );
    let healed = Arc::new(RecordingCalendarApi::with_events(client.events()));
    let recovered = state.clone().with_calendar_api(healed.clone());
    let remaining = preview(&recovered).await;
    let completed = approve(&recovered, &remaining, "automation_worker").await;
    assert_eq!(completed["status"], "applied");
    assert_eq!(healed.recorded_calls().len(), 1);
    assert!(
        matches!(&healed.recorded_calls()[0],RecordedCalendarCall::InsertEvent{event} if event.external_id==failed)
    );
    assert_eq!(preview(&recovered).await["operations"], json!([]));
}

// P1B-43: the export is performed by the automation worker whoever approves.

async fn boundary_rows(state: &AppState) -> Vec<Value> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT object_refs_json, payload_json, provenance_json FROM logs WHERE event_type='compartment_boundary_decided' ORDER BY rowid",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .unwrap();
    rows.into_iter()
        .map(|(refs, payload, provenance)| {
            json!({
                "object_refs": serde_json::from_str::<Value>(&refs).unwrap(),
                "payload": serde_json::from_str::<Value>(&payload).unwrap(),
                "provenance": serde_json::from_str::<Value>(&provenance).unwrap(),
            })
        })
        .collect()
}

#[tokio::test]
async fn user_approval_applies_every_operation_to_the_client() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    assert_eq!(p["operations"].as_array().unwrap().len(), 3);
    assert!(client.recorded_calls().is_empty());
    let (status, applied) = request(&state,"POST",APPROVE,json!({"schema_version":SCHEMA,"preview_id":p["preview_id"],"authority_source":"user","export_mode":"mock"})).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["status"], "applied", "{applied}");
    // The response alone proves nothing; the client must have been called.
    let calls = serde_json::to_value(client.recorded_calls()).unwrap();
    assert_eq!(calls, expected_inserts(&p));
    assert_eq!(calls.as_array().unwrap().len(), 3);
    println!("P1B43_CALLS_1 {calls}");
    assert_eq!(
        applied["applied_events"],
        serde_json::to_value(client.events()).unwrap()
    );
    assert_eq!(applied["diagnostics"], json!([]));
    assert_eq!(preview(&state).await["operations"], json!([]));
}

#[tokio::test]
async fn user_approved_results_match_what_the_github_path_reports() {
    let (state, client) = setup().await;
    let p = preview(&state).await;
    let applied = approve(&state, &p, "user").await;
    let outcomes = applied["operation_results"].as_array().unwrap();
    assert_eq!(outcomes.len(), 3);
    for (outcome, operation) in outcomes.iter().zip(p["operations"].as_array().unwrap()) {
        assert_eq!(outcome["status"], "applied", "{outcome}");
        assert!(outcome["operation_id"]
            .as_str()
            .unwrap()
            .ends_with(operation["event"]["external_id"].as_str().unwrap()));
        // Nothing in a Calendar result attributes the export to the approver.
        assert!(!outcome.to_string().contains("\"user\""), "{outcome}");
    }
    println!("P1B43_RESULTS_2 {}", applied["operation_results"]);
    assert_eq!(client.recorded_calls().len(), 3);

    // The same approver on the GitHub path, in the same process.
    let github_preview = ok(&state,"POST","/projection/preview",json!({"schema_version":"ubu.orchestrator.projection_preview.v1","owner":"UbU-project","repo":"ubu-orchestrator","issue_number":7,"observed_labels":[],"desired_labels":["ubu-managed"],"existing_repository_labels":["ubu","ubu-managed"]})).await;
    let github = ok(&state,"POST","/projection/approve",json!({"schema_version":"ubu.orchestrator.projection_approval.v1","preview_id":github_preview["preview_id"],"approved":true,"authority_source":"user"})).await;
    assert_eq!(github["status"], "applied", "{github}");
    assert_eq!(
        github["operation_results"][0]["authority_source"],
        "automation_worker"
    );
    println!("P1B43_GITHUB_RESULTS_2 {}", github["operation_results"]);

    // Both paths recorded the automation worker at the boundary for a `user` approve.
    let logged: Vec<Value> = boundary_rows(&state)
        .await
        .into_iter()
        .map(|row| row["payload"]["authority_source"].clone())
        .collect();
    assert_eq!(logged, vec![json!("automation_worker"); 4]);
}

#[tokio::test]
async fn user_approval_is_logged_at_the_boundary_as_the_automation_worker() {
    let (state, _client) = setup().await;
    let p = preview(&state).await;
    let applied = approve(&state, &p, "user").await;
    let rows = boundary_rows(&state).await;
    assert_eq!(rows.len(), 3);
    for (row, outcome) in rows.iter().zip(applied["operation_results"].as_array().unwrap()) {
        // The entry is for the same operation the result reports as applied.
        assert_eq!(
            row["object_refs"],
            json!([p["preview_id"], outcome["operation_id"]])
        );
        assert_eq!(outcome["status"], "applied");
        assert_eq!(row["payload"]["adjudication_result"], "accepted");
        assert_eq!(row["payload"]["authority_source"], "automation_worker");
        assert_eq!(
            row["payload"]["provenance"]["authority_source"],
            "automation_worker"
        );
        assert_eq!(
            row["payload"]["provenance"]["source"]["source_kind"],
            "google_calendar"
        );
        assert_eq!(row["provenance"]["authority_source"], "automation_worker");
        assert!(!row.to_string().contains("\"user\""), "{row}");
    }
    println!("P1B43_BOUNDARY_3 {}", json!(rows));
}

#[tokio::test]
async fn no_external_export_still_refuses_a_user_approval() {
    let (state, client) = setup().await;
    let p = ok(
        &state,
        "GET",
        "/projection/calendar/preview?no_external_export=true",
        Value::Null,
    )
    .await;
    let applied = approve(&state, &p, "user").await;
    assert_eq!(applied["status"], "failed");
    assert!(client.recorded_calls().is_empty());
    assert!(applied["operation_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["status"] == "skipped"));
    let diagnostics = applied["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 3);
    assert!(diagnostics
        .iter()
        .all(|d| d["code"] == "calendar_export_rejected"));
    let logs = boundaries(&state).await;
    assert_eq!(logs.len(), 3);
    // Refused by the policy member, and not for the approver's authority.
    assert!(logs.iter().all(|l| l["adjudication_result"] == "rejected"
        && l["member_evaluated"] == "no_external_export"
        && l["authority_source"] == "automation_worker"
        && l["reason"].as_str().unwrap().contains("no_external_export")
        && !l["reason"].as_str().unwrap().contains("user-equivalent")));
    assert_eq!(results(&state).await[0]["applied_events"], json!([]));
    assert_eq!(
        preview(&state).await["operations"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn a_permit_for_another_operation_is_an_internal_error() {
    use axum::response::IntoResponse;
    use ubu_core::{
        projection::{ExportProjectionContext, Legitimizer},
        AuthoritySource, ObjectRef, Provenance,
    };
    use ubu_orchestrator::errors::AppError;

    let (state, client) = setup().await;
    let p = preview(&state).await;
    let stored = calendar_apply::load_preview(
        state.inner().store.pool(),
        p["preview_id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    let first = calendar_apply::lower_operation(&stored.operations[0]).unwrap();
    let second = calendar_apply::lower_operation(&stored.operations[1]).unwrap();
    assert_ne!(first.operation_id, second.operation_id);

    let now = UbuTimestamp::parse(NOW).unwrap();
    let compartment = ObjectRef {
        id: UbuId::new(ObjectType::Compartment),
        object_type: ObjectType::Compartment,
    };
    let actor = ObjectRef {
        id: UbuId::new(ObjectType::Identity),
        object_type: ObjectType::Identity,
    };
    let provenance = Provenance {
        created_at: now,
        created_by: None,
        authority_source: AuthoritySource::AutomationWorker,
        source: Some(first.target.clone()),
        source_refs: None,
    };
    let gate = Legitimizer::gate_export_projection(ExportProjectionContext {
        operation: &first,
        effective_policy: stored.policy_summary.as_ref(),
        compartment_ref: &compartment,
        actor_identity_ref: &actor,
        authority_source: AuthoritySource::AutomationWorker,
        effective_time: now,
        provenance: &provenance,
    });
    let permit = gate.permit().expect("the worker is permitted");

    calendar_apply::ensure_permit_matches(permit, &first).expect("its own operation");
    let error = calendar_apply::ensure_permit_matches(permit, &second).unwrap_err();
    assert!(
        matches!(&error, AppError::Internal(message) if message == "Calendar export permit does not match the operation"),
        "{error:?}"
    );
    assert_eq!(
        error.into_response().status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(client.recorded_calls().is_empty());
}
