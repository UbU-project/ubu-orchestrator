use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{id_registry::ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    planning_time::{timestamp_at, FixedClock},
    services::planning_service,
    state::AppState,
};
use ubu_store::{
    models::{
        calendar_record::NewCalendarRecord, log_record::NewLogRecord,
        object_record::NewObjectRecord,
    },
    queries,
};

const NOW: &str = "2026-06-10T09:00:00Z";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn post(state: &AppState, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(json_request(uri, body))
        .await
        .unwrap();
    let status = response.status();
    (status, json_body(response).await)
}
async fn generate(state: &AppState, body: Value) -> Value {
    let (status, body) = post(state, "/planning/generate", body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["plan"].is_object(), "{body}");
    body
}
async fn calendar(state: &AppState) -> Value {
    let response = build_router(state.clone())
        .oneshot(get_request("/calendar/current"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}
async fn next(state: &AppState) -> Value {
    let uri = format!(
        "/next-action?schema_version={}",
        ubu_orchestrator::api::next_action::NEXT_ACTION_SCHEMA_VERSION
    );
    let response = build_router(state.clone())
        .oneshot(get_request(&uri))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}
fn step<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task_id"] == id)
        .unwrap()
}
fn fixed(start: &str, end: &str) -> Value {
    json!({"static_window":{"start":start,"end":end}})
}

#[tokio::test]
async fn durations_reach_kernel_and_calendar_in_seconds() {
    let state = state().await;
    let fixed_id = admit_task(
        &state,
        "Fixed",
        json!({"duration_estimate":{"type":"fixed","seconds":1800}}),
    )
    .await;
    let legacy = admit_task(&state, "Legacy minutes", json!({"duration_minutes":15})).await;
    let stochastic = admit_task(
        &state,
        "Stochastic",
        json!({"duration_estimate":{
        "type":"shifted_lognormal_p95","min_seconds":300,"mode_seconds":900,"p95_seconds":3600}}),
    )
    .await;
    let legacy_estimate =
        admit_task(&state, "Estimate minutes", json!({"estimate_minutes":2})).await;
    let seconds = admit_task(
        &state,
        "Estimate seconds",
        json!({"estimate":{"seconds":47}}),
    )
    .await;
    let default = admit_task(&state, "Default", json!({})).await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let kernel = ubu_planning_core::PlanningRequest::from(request);
    for (id, expected) in [
        (&fixed_id, 1800),
        (&legacy, 900),
        (&legacy_estimate, 120),
        (&seconds, 47),
        (&default, 1800),
    ] {
        assert_eq!(
            kernel
                .tasks()
                .iter()
                .find(|t| &t.id == id)
                .unwrap()
                .duration,
            ubu_planning_core::DurationModel::Fixed { seconds: expected }
        );
    }
    assert_eq!(
        kernel
            .tasks()
            .iter()
            .find(|t| t.id == stochastic)
            .unwrap()
            .duration,
        ubu_planning_core::DurationModel::ShiftedLognormalP95 {
            min_seconds: 300,
            mode_seconds: 900,
            p95_seconds: 3600
        }
    );
    generate(&state, json!({})).await;
    let cal = calendar(&state).await;
    for (id, expected) in [(&fixed_id, 1800), (&legacy, 900)] {
        let step = step(&cal, id);
        assert_eq!(
            step["end"].as_u64().unwrap() - step["start"].as_u64().unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn dependency_slack_and_deadline_coordinates_are_seconds() {
    for (gap, fragile) in [(600, false), (240, true)] {
        let state = state().await;
        let a = admit_task(&state, "A", fixed(NOW, "2026-06-10T09:01:00Z")).await;
        let start = timestamp_seconds(NOW) + 60 + gap;
        let end = start + 60;
        let mut b = fixed(&timestamp_at(start).unwrap(), &timestamp_at(end).unwrap());
        b["depends_on"] = json!([a]);
        b["due_at"] = json!(timestamp_at(end + gap).unwrap());
        let b = admit_task(&state, "B", b).await;
        let response = generate(&state, json!({})).await;
        let findings = response["risk_report"]["findings"].as_array().unwrap();
        assert_eq!(
            findings
                .iter()
                .any(|f| f["category"] == "dependency_fragility"),
            fragile,
            "{response}"
        );
        let deadline = findings
            .iter()
            .find(|f| f["category"] == "deadline_risk" && f["subject_ref"] == b);
        assert_eq!(deadline.is_some(), fragile);
        if let Some(finding) = deadline {
            assert!(finding["detail"].as_str().unwrap().contains("240 seconds"));
        }
        assert_eq!(
            response["human_complete_plan_quality"]["feedback_latency"],
            end - timestamp_seconds(NOW)
        );
    }
}

#[tokio::test]
async fn kernel_legitimization_detects_seconds_of_staleness() {
    for (observed, stale) in [
        ("2026-06-10T08:59:00Z", true),
        ("2026-06-10T08:59:45Z", false),
    ] {
        let state = state().await;
        admit_task(
            &state,
            "Static",
            fixed("2026-06-10T09:00:30Z", "2026-06-10T09:05:00Z"),
        )
        .await;
        admit_preference(&state, "affect_freshness_seconds", json!(60)).await;
        admit_snapshot(
            &state,
            observed,
            json!({"energy":8.0,"stress":2.0,"mood_intensity":2.0}),
        )
        .await;
        let request = planning_service::build_request_from_store(&state)
            .await
            .unwrap();
        let observation = &request.affect_observation.as_ref().unwrap().dimensions["energy"];
        assert_eq!(observation.observed_at, timestamp_seconds(observed));
        assert_eq!(observation.source_kind, "live_observation");
        // At H.start the observation is <=60 seconds old; at the anchor it can be >60.
        let response = generate(&state, json!({})).await;
        let dimensions = response["legitimization"]["stale_dimensions"]
            .as_array()
            .unwrap();
        assert_eq!(dimensions.contains(&json!("energy")), stale, "{response}");
    }
}

#[tokio::test]
async fn exact_static_seconds_and_timestamp_round_trip() {
    let state = state().await;
    let id = admit_task(
        &state,
        "Exact static",
        fixed("2026-06-10T09:00:30Z", "2026-06-10T09:05:00Z"),
    )
    .await;
    generate(&state, json!({})).await;
    let cal = calendar(&state).await;
    let step = step(&cal, &id);
    assert_eq!(step["start"], timestamp_seconds("2026-06-10T09:00:30Z"));
    assert_eq!(step["end"], timestamp_seconds("2026-06-10T09:05:00Z"));
    for (numeric, string) in [("start", "start_at"), ("end", "end_at")] {
        assert_eq!(
            step[numeric],
            timestamp_seconds(step[string].as_str().unwrap())
        );
        assert!(step[string].as_str().unwrap().ends_with('Z'));
    }
    assert_eq!(cal["steps"], cal["selected_candidate"]["steps"]);
    println!(
        "P1B15_STEP_BEGIN\n{}\nP1B15_STEP_END",
        serde_json::to_string_pretty(step).unwrap()
    );
}

#[tokio::test]
async fn default_horizon_starts_now_and_keeps_in_progress_static_whole() {
    let state = state().await;
    let dynamic = admit_task(&state, "Dynamic", json!({"duration_minutes":15})).await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let h = request.time_window.unwrap();
    assert_eq!(
        (h.start, h.end),
        (timestamp_seconds(NOW), timestamp_seconds(NOW) + 86400)
    );
    let fixed_id = admit_task(
        &state,
        "In progress",
        fixed("2026-06-10T08:55:00Z", "2026-06-10T09:05:00Z"),
    )
    .await;
    generate(&state, json!({})).await;
    let cal = calendar(&state).await;
    assert_eq!(
        step(&cal, &fixed_id)["start"],
        timestamp_seconds("2026-06-10T08:55:00Z")
    );
    assert!(
        step(&cal, &dynamic)["start"].as_u64().unwrap()
            >= timestamp_seconds("2026-06-10T09:05:00Z")
    );
}

#[tokio::test]
async fn explicit_horizon_beats_stored_and_stored_beats_default() {
    let state = state().await;
    let id = admit_task(&state, "Dynamic", json!({"duration_minutes":1})).await;
    store_calendar_window(&state, "2026-06-10T08:00:00Z", "2026-06-10T10:00:00Z").await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        request.time_window.as_ref().unwrap().start,
        timestamp_seconds("2026-06-10T08:00:00Z")
    );
    assert_eq!(
        request.time_window.as_ref().unwrap().end,
        timestamp_seconds("2026-06-10T10:00:00Z")
    );
    assert_eq!(
        request.tasks[0].window.as_ref().unwrap().start,
        timestamp_seconds(NOW)
    );
    generate(&state, json!({})).await;
    assert!(
        step(&calendar(&state).await, &id)["start"]
            .as_u64()
            .unwrap()
            >= timestamp_seconds(NOW)
    );
    generate(
        &state,
        json!({"horizon":{"start":"2026-06-10T10:00:00Z","end":"2026-06-10T11:00:00Z"}}),
    )
    .await;
    let cal = calendar(&state).await;
    assert!(
        step(&cal, &id)["start"].as_u64().unwrap() >= timestamp_seconds("2026-06-10T10:00:00Z")
    );
    assert!(step(&cal, &id)["end"].as_u64().unwrap() <= timestamp_seconds("2026-06-10T11:00:00Z"));
}

#[tokio::test]
async fn invalid_horizon_is_400_but_full_requests_ignore_it() {
    let state = state().await;
    admit_task(&state, "Task", json!({})).await;
    for (start, end) in [(NOW, NOW), ("2026-06-11T09:00:00Z", NOW), ("bad", NOW)] {
        let (status, body) = post(
            &state,
            "/planning/generate",
            json!({"horizon":{"start":start,"end":end}}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["diagnostics"][0]["code"], "invalid_horizon");
    }
    let response=generate(&state,json!({"horizon":{"start":"bad","end":"bad"},"request":{
        "schema_version":ubu_planning_core::PLANNING_SCHEMA_VERSION,"request_id":"explicit-units",
        "time_window":{"start":5,"end":25},"tasks":[{"id":"unit-agnostic","duration":10}]
    }})).await;
    let step = &response["plan"]["steps"][0];
    assert_eq!(
        step["end"].as_u64().unwrap() - step["start"].as_u64().unwrap(),
        10
    );
    assert!(step["start"].as_u64().unwrap() >= 5 && step["end"].as_u64().unwrap() <= 25);
}

#[tokio::test]
async fn next_action_skips_ended_steps_and_reports_stale_calendar() {
    let state = state().await;
    // Stored scope keeps past Statics so the selector must filter them itself.
    store_calendar_window(&state, "2026-06-10T08:00:00Z", "2026-06-10T11:00:00Z").await;
    admit_task(&state, "Ended", fixed("2026-06-10T08:50:00Z", NOW)).await;
    let progress = admit_task(&state, "In progress", fixed(NOW, "2026-06-10T09:05:00Z")).await;
    let later = admit_task(
        &state,
        "Later",
        fixed("2026-06-10T09:10:00Z", "2026-06-10T09:15:00Z"),
    )
    .await;
    generate(&state, json!({})).await;
    let during = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-06-10T09:02:00Z").unwrap(),
    ));
    assert_eq!(next(&during).await["recommendation"]["task_id"], progress);
    let after = state.clone().with_clock(FixedClock(
        UbuTimestamp::parse("2026-06-10T09:05:00Z").unwrap(),
    ));
    assert_eq!(next(&after).await["recommendation"]["task_id"], later);
    let ended = state.with_clock(FixedClock(
        UbuTimestamp::parse("2026-06-10T09:15:00Z").unwrap(),
    ));
    let response = next(&ended).await;
    assert!(response["recommendation"].is_null());
    assert_eq!(response["diagnostics"][0]["code"], "stale_calendar");
    assert_eq!(
        response["diagnostics"][0]["message"],
        "the current Calendar has no placement after now; regenerate the plan"
    );
    // The Calendar diagnostic also applies after every placed Task completes.
    for step in calendar(&ended).await["steps"].as_array().unwrap() {
        let id = step["task_id"].as_str().unwrap();
        let (status, body) = post(
            &ended,
            &format!("/task/{id}/action"),
            json!({
                "schema_version":ubu_orchestrator::api::user_action::TASK_ACTION_SCHEMA_VERSION,
                "action":"complete"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        next(&ended).await["diagnostics"][0]["code"],
        "stale_calendar"
    );
}

#[tokio::test]
async fn completed_static_prerequisite_constrains_neither_static_nor_dynamic() {
    let state = state().await;
    let prior = admit_task(&state, "Completed", fixed(NOW, "2026-06-10T09:10:00Z")).await;
    let (status,body)=post(&state,&format!("/task/{prior}/action"),json!({
        "schema_version":ubu_orchestrator::api::user_action::TASK_ACTION_SCHEMA_VERSION,"action":"complete"
    })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut payload = fixed("2026-06-10T09:00:30Z", "2026-06-10T09:05:00Z");
    payload["blocked_by"] = json!([prior]);
    admit_task(&state, "Static dependent", payload).await;
    let dynamic = admit_task(
        &state,
        "Dynamic dependent",
        json!({"duration_minutes":1,"blocked_by":[prior]}),
    )
    .await;
    let request = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    assert_eq!(
        request
            .tasks
            .iter()
            .find(|t| t.id == dynamic)
            .unwrap()
            .window
            .as_ref()
            .unwrap()
            .start,
        timestamp_seconds(NOW)
    );
    assert!(request.tasks.iter().all(|t| t.depends_on.is_empty()));
    let response = generate(&state, json!({})).await;
    assert!(!response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "static_task_collision"));
}

#[tokio::test]
async fn repair_horizon_is_never_earlier_than_clock_or_frozen_end() {
    let state = state().await;
    store_calendar_window(&state, NOW, "2026-06-10T12:00:00Z").await;
    let frozen = admit_task(&state, "Frozen", fixed(NOW, "2026-06-10T09:10:00Z")).await;
    let dynamic = admit_task(&state, "Dynamic", json!({"duration_minutes":10})).await;
    generate(&state, json!({})).await;
    append_user_override_log(&state, &frozen).await;
    let state = state.with_clock(FixedClock(
        UbuTimestamp::parse("2026-06-10T10:00:00Z").unwrap(),
    ));
    let (status, response) = post(
        &state,
        "/planning/recalculate",
        json!({
            "triggered_at":"2026-06-10T10:00:00Z","trigger_type":"worker_request","objects":[]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        step(&response["plan"], &frozen)["start"],
        timestamp_seconds(NOW)
    );
    assert!(
        step(&response["plan"], &dynamic)["start"].as_u64().unwrap()
            >= timestamp_seconds("2026-06-10T10:00:00Z")
    );
}

#[test]
fn horizon_environment_is_validated_without_global_test_env_races() {
    for (value, expected) in [
        (None, Some(86400)),
        (Some("3600"), Some(3600)),
        (Some("2678400"), Some(2678400)),
        (Some("0"), None),
        (Some("-1"), None),
        (Some("2678401"), None),
        (Some("bad"), None),
        (Some(""), None),
    ] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "horizon_env_child", "--nocapture"])
            .env(
                "P1B15_ENV_EXPECTED",
                expected
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "invalid".into()),
            );
        if let Some(value) = value {
            command.env("UBU_PLANNING_HORIZON_SECONDS", value);
        } else {
            command.env_remove("UBU_PLANNING_HORIZON_SECONDS");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn horizon_env_child() {
    let Ok(expected) = std::env::var("P1B15_ENV_EXPECTED") else {
        return;
    };
    let state = AppState::in_memory(ServerConfig::from_env()).await;
    if expected == "invalid" {
        assert!(state
            .err()
            .expect("startup must fail")
            .to_string()
            .contains("UBU_PLANNING_HORIZON_SECONDS"));
    } else {
        let state = state
            .unwrap()
            .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()));
        let request = planning_service::build_request_from_store(&state)
            .await
            .unwrap();
        let h = request.time_window.unwrap();
        assert_eq!(h.start, timestamp_seconds(NOW));
        assert_eq!(h.end - h.start, expected.parse::<u64>().unwrap());
    }
}

#[test]
fn timestamps_reject_unrepresentable_coordinates_and_normalize_utc() {
    assert!(timestamp_at(u64::MAX).is_err());
    assert_eq!(timestamp_at(0).unwrap(), "1970-01-01T00:00:00Z");
    assert_eq!(
        timestamp_at(timestamp_seconds("2026-06-10T10:00:30+01:00")).unwrap(),
        "2026-06-10T09:00:30Z"
    );
}

async fn admit_task(state: &AppState, title: &str, extra: Value) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let mut payload = json!({
        "id": id,
        "title": title,
        "status": "active",
        "provenance": {
            "created_at": now,
            "authority_source": "user",
            "source": {
                "source_kind": "test",
                "source_id": title
            }
        }
    });
    let map = payload.as_object_mut().expect("object");
    for (key, value) in extra.as_object().expect("extra object") {
        map.insert(key.clone(), value.clone());
    }

    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            ubu_core::AuthoritySource::User,
            UbuTimestamp::parse(&now).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Task.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("task admitted");
    id
}

async fn admit_preference(state: &AppState, name: &str, value: Value) -> String {
    let id = UbuId::new(ObjectType::Preference).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            ubu_core::AuthoritySource::User,
            UbuTimestamp::parse(&now).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Preference.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload: json!({
                "id": id,
                "name": name,
                "value": value,
                "authority_source": "user",
                "provenance": {
                    "created_at": now,
                    "authority_source": "user",
                    "source": {
                        "source_kind": "test",
                        "source_id": name
                    }
                }
            }),
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("preference admitted");
    id
}

async fn admit_snapshot(state: &AppState, observed_at: &str, values: Value) -> String {
    let id = UbuId::new(ObjectType::Snapshot).to_string();
    let envelope = state
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            ubu_core::AuthoritySource::User,
            UbuTimestamp::parse(observed_at).unwrap(),
        )
        .unwrap();
    queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::Snapshot.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload: json!({
                "id": id,
                "captured_at": observed_at,
                "objects": [],
                "affect": {
                    "source_kind": "live_observation",
                    "observed_at": observed_at,
                    "dimensions": {
                        "energy": snapshot_dimension(
                            "energy",
                            "higher_is_better",
                            values["energy"].as_f64().expect("energy")
                        ),
                        "stress": snapshot_dimension(
                            "stress",
                            "lower_is_better",
                            values["stress"].as_f64().expect("stress")
                        ),
                        "mood_intensity": snapshot_dimension(
                            "mood_intensity",
                            "lower_is_better",
                            values["mood_intensity"].as_f64().expect("mood intensity")
                        )
                    }
                }
            }),
            created_at: observed_at.to_owned(),
            updated_at: observed_at.to_owned(),
        },
    )
    .await
    .expect("snapshot admitted");
    id
}

fn snapshot_dimension(dimension: &str, direction: &str, value: f64) -> Value {
    json!({
        "dimension": dimension,
        "direction": direction,
        "value": value,
        "scale": {"min": 0, "max": 10},
        "threshold": {"warning_delta": 1.0, "critical_delta": 2.0}
    })
}

async fn store_calendar_window(state: &AppState, start: &str, end: &str) {
    queries::store_calendar(
        state.inner().store.pool(),
        NewCalendarRecord {
            id: UbuId::new(ObjectType::Calendar).to_string(),
            plan_id: UbuId::new(ObjectType::Plan).to_string(),
            window_start: start.to_owned(),
            window_end: end.to_owned(),
            payload: json!({
                "windows": [{"start": start, "end": end}]
            }),
            created_at: "2026-06-10T14:30:00Z".to_owned(),
        },
    )
    .await
    .expect("calendar stored");
}

async fn append_user_override_log(state: &AppState, task_id: &str) {
    let now = UbuTimestamp::now_utc().to_string();
    let envelope = state
        .envelope_for(
            Default::default(),
            ubu_core::AuthoritySource::UserOverride,
            UbuTimestamp::parse(&now).unwrap(),
        )
        .unwrap();
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "decision_recorded".to_owned(),
            object_refs: json!([task_id]),
            payload: json!({"action": "override"}),
            provenance: json!({
                "created_at": now,
                "authority_source": "user_override"
            }),
            created_at: now,
        },
    )
    .await
    .expect("override log");
}

fn timestamp_seconds(value: &str) -> u64 {
    UbuTimestamp::parse(value)
        .expect("timestamp")
        .inner()
        .unix_timestamp() as u64
}

fn json_request(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}
