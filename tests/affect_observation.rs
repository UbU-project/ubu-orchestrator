use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    build_router, config::ServerConfig, planning_time::FixedClock, services::planning_service,
    state::AppState,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

const NOW: &str = "2026-06-10T09:00:00Z";
const SCHEMA: &str = "ubu.orchestrator.affect_observation.v1";

async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn record(state: &AppState, energy: f64) -> Value {
    let (status, body) = call(
        state,
        "POST",
        "/affect/observation",
        Some(json!({
            "schema_version":SCHEMA,"energy":energy,"stress":3,"mood_intensity":3
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}
async fn admit(state: &AppState, kind: ObjectType, payload: Value, updated_at: &str) {
    let id = UbuId::parse(payload["id"].as_str().unwrap()).unwrap();
    let now = state.planning_now();
    let envelope = state
        .envelope_for(
            [(id.clone(), VersionRef::Absent)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: kind.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "synthetic-test".into(),
            payload,
            created_at: NOW.into(),
            updated_at: updated_at.into(),
        },
    )
    .await
    .unwrap();
}
async fn task(state: &AppState) {
    let (status, body) = call(state,"POST","/task",Some(json!({
        "schema_version":"ubu.orchestrator.task_capture.v1","title":"Invented affect observation control",
        "duration_estimate":{"type":"fixed","seconds":900}
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}
async fn plan(state: &AppState) -> Value {
    let (status, body) = call(
        state,
        "POST",
        "/planning/generate",
        Some(json!({
            "schema_version":"planning-kernel-contract/0.1","request":null
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn refusals_name_schema_missing_dimension_and_invalid_value_without_writes() {
    let state = state().await;
    let valid = json!({"schema_version":SCHEMA,"energy":7,"stress":3,"mood_intensity":3});
    let mut cases = Vec::new();
    let mut unknown = valid.clone();
    unknown["schema_version"] = json!("invented-invalid");
    cases.push((unknown, "unknown_schema_version"));
    let mut absent_schema = valid.clone();
    absent_schema
        .as_object_mut()
        .unwrap()
        .remove("schema_version");
    cases.push((absent_schema, "missing_schema_version"));
    for name in ["energy", "stress", "mood_intensity"] {
        let mut body = valid.clone();
        body.as_object_mut().unwrap().remove(name);
        cases.push((body, "affect_dimension_missing"));
        for value in [
            json!(-1),
            json!(11),
            json!("invented-not-a-number"),
            json!([]),
        ] {
            let mut body = valid.clone();
            body[name] = value;
            cases.push((body, "affect_value_out_of_range"));
        }
    }
    for (request, code) in cases {
        let (status, body) = call(&state, "POST", "/affect/observation", Some(request)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["diagnostics"][0]["code"], code, "{body}");
    }
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type = 'Snapshot'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        call(&state, "GET", "/affect/observation", None).await.1["observation"],
        Value::Null
    );
}

#[tokio::test]
async fn recorded_snapshot_is_user_stamped_immutable_and_reaches_the_kernel_without_recalculation()
{
    let state = state().await;
    let written = record(&state, 7.25).await;
    assert_eq!(written["source_kind"], "live_observation");
    assert_eq!(written["dimension_count"], 3);
    assert_eq!(written["observed_at"], NOW);
    let row = queries::get_current_state(
        state.inner().store.pool(),
        written["snapshot_id"].as_str().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(row.object_type, "Snapshot");
    assert_eq!(row.version, 1);
    assert_eq!(row.status, "active");
    assert_eq!(row.compartment_label, "user-capture");
    let snapshot: ubu_core::core::Snapshot = serde_json::from_str(&row.payload_json).unwrap();
    assert_eq!(snapshot.captured_at.to_string(), NOW);
    assert!(snapshot.objects.is_empty());
    assert_eq!(snapshot.affect.unwrap().dimensions.energy.value, 7.25);
    let (status, read) = call(&state, "GET", "/affect/observation", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["observation"]["snapshot_id"], written["snapshot_id"]);
    assert_eq!(
        read["observation"]["dimensions"],
        json!({"energy":7.25,"stress":3.0,"mood_intensity":3.0})
    );
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let observation = request.affect_observation.unwrap();
    let observed_at = UbuTimestamp::parse(NOW).unwrap().inner().unix_timestamp() as u64;
    for (name, value) in [("energy", 7.25), ("stress", 3.0), ("mood_intensity", 3.0)] {
        assert_eq!(observation.dimensions[name].value, value);
        assert_eq!(observation.dimensions[name].source_kind, "live_observation");
        assert_eq!(observation.dimensions[name].observed_at, observed_at);
    }
    let plans: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans")
        .fetch_one(state.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(plans, 0);
    for method in ["PATCH", "DELETE"] {
        assert_eq!(
            call(&state, method, "/affect/observation", None).await.0,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    for field in ["source_kind", "observed_at", "snapshot_id"] {
        let mut body = json!({"schema_version":SCHEMA,"energy":7,"stress":3,"mood_intensity":3});
        body[field] = json!("invented-client-attribution");
        assert_eq!(
            call(&state, "POST", "/affect/observation", Some(body))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[tokio::test]
async fn second_check_in_wins_even_at_the_same_clock_time_and_keeps_the_first_snapshot() {
    let state = state().await;
    let first = record(&state, 7.0).await;
    let second = record(&state, 6.0).await;
    assert_ne!(first["snapshot_id"], second["snapshot_id"]);
    let read = call(&state, "GET", "/affect/observation", None).await.1;
    assert_eq!(read["observation"]["snapshot_id"], second["snapshot_id"]);
    assert_eq!(read["observation"]["dimensions"]["energy"], 6.0);
    let first_row = queries::get_current_state(
        state.inner().store.pool(),
        first["snapshot_id"].as_str().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first_row.version, 1);
    assert_eq!(
        serde_json::from_str::<Value>(&first_row.payload_json).unwrap()["affect"]["dimensions"]
            ["energy"]["value"],
        7.0
    );
    assert_eq!(
        planning_service::build_request_from_store(&state)
            .await
            .unwrap()
            .affect_observation
            .unwrap()
            .dimensions["energy"]
            .value,
        6.0
    );
}

#[tokio::test]
async fn default_priors_warn_on_low_energy_but_keep_a_plan_and_live_report_figures() {
    let state = state().await;
    task(&state).await;
    record(&state, 2.0).await;
    let body = plan(&state).await;
    assert!(body["plan"].is_object(), "{body}");
    assert_eq!(body["legitimization"]["mode"], "warn_only");
    assert_eq!(body["legitimization"]["affect_feasible"], false);
    assert!(body["legitimization"]["violated_dimensions"]
        .as_array()
        .unwrap()
        .contains(&json!("energy")));
    assert!(!body["legitimization"]["stale_affect_warning"]
        .as_str()
        .unwrap()
        .contains("bootstrap default profile observation"));
    let categories: Vec<_> = body["risk_report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["category"].as_str().unwrap())
        .collect();
    assert!(categories.contains(&"affect_margin"), "{body}");
    assert_eq!(
        body["human_complete_plan_quality"]["post_plan_state_delta"],
        "at_risk"
    );
    assert!(!body["human_complete_plan_quality"]["revision_suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value
            .as_str()
            .unwrap()
            .starts_with("Record how you are feeling:")));
}

#[tokio::test]
async fn comfortable_reading_has_measurements_without_any_affect_finding_or_stand_in_sentence() {
    let state = state().await;
    task(&state).await;
    record(&state, 7.0).await;
    let body = plan(&state).await;
    assert!(body["plan"].is_object(), "{body}");
    assert_eq!(body["legitimization"]["affect_feasible"], true);
    assert!(
        body["human_complete_plan_quality"]["affect_margin"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    for finding in body["risk_report"]["findings"].as_array().unwrap() {
        assert!(![
            "affect_margin",
            "post_plan_depletion",
            "destructive_pressure"
        ]
        .contains(&finding["category"].as_str().unwrap()));
    }
    assert!(body["human_complete_plan_quality"]["revision_suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|value| !value
            .as_str()
            .unwrap()
            .starts_with("Record how you are feeling:")));
    assert!(["better", "neutral", "depleted", "at_risk"].contains(
        &body["human_complete_plan_quality"]["post_plan_state_delta"]
            .as_str()
            .unwrap()
    ));
}

#[tokio::test]
async fn a_newer_snapshot_without_affect_cannot_hide_the_recorded_observation() {
    let state = state().await;
    let written = record(&state, 7.0).await;
    admit(&state,ObjectType::Snapshot,json!({"id":UbuId::new(ObjectType::Snapshot),"captured_at":"2026-06-10T09:01:00Z","objects":[]}),"2026-06-10T09:01:00Z").await;
    assert_eq!(
        call(&state, "GET", "/affect/observation", None).await.1["observation"]["snapshot_id"],
        written["snapshot_id"]
    );
    assert_eq!(
        planning_service::build_request_from_store(&state)
            .await
            .unwrap()
            .affect_observation
            .unwrap()
            .dimensions["energy"]
            .value,
        7.0
    );
}

#[tokio::test]
async fn any_named_calibration_setting_enforces_even_when_its_value_is_a_default_or_unparseable() {
    for (name, value) in [
        ("acceptable_energy_floor", json!(4)),
        ("affect_energy_floor", json!(4)),
        ("energy_floor", Value::Null),
        ("tolerable_stress_ceiling", json!(7)),
        ("affect_stress_ceiling", json!(7)),
        ("stress_ceiling", json!("invented-unparseable")),
        ("tolerable_intensity_ceiling", json!(8)),
        ("tolerable_mood_intensity_ceiling", json!(8)),
        ("affect_mood_intensity_ceiling", json!(8)),
        ("mood_intensity_ceiling", json!(8)),
    ] {
        let state = state().await;
        task(&state).await;
        record(&state, 2.0).await;
        admit(&state,ObjectType::Setting,json!({"id":UbuId::new(ObjectType::Setting),"name":name,"value":value,"authority_source":"user","provenance":{"created_at":NOW,"authority_source":"user"}}),NOW).await;
        let request = planning_service::build_request_from_store(&state)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(request.affect_profile.unwrap()).unwrap()["mode"],
            "enforce",
            "{name}"
        );
        assert!(
            plan(&state).await["plan"].is_null(),
            "{name}: a calibrated floor still enforces"
        );
    }
}
