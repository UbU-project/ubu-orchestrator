use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use ubu_core::id_registry::ObjectType;
use ubu_core::{UbuId, UbuTimestamp};
use ubu_orchestrator::api::next_action::NEXT_ACTION_SCHEMA_VERSION;
use ubu_orchestrator::api::planning::{
    AffectLegitimizationModeBody, PlanningModeBody, ProbabilityQualityBody,
};
use ubu_orchestrator::api::recalculation::RECALCULATION_SCHEMA_VERSION;
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::services::planning_service;
use ubu_orchestrator::state::AppState;
use ubu_planning_core::{DurationModel, PlanningRequest, PLANNING_SCHEMA_VERSION};
use ubu_store::models::calendar_record::NewCalendarRecord;
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

#[test]
fn probability_quality_full_round_trips_and_estimated_is_rejected() {
    let encoded = serde_json::to_string(&ProbabilityQualityBody::Full).expect("serialize full");
    assert_eq!(encoded, r#""full""#);
    assert_eq!(
        serde_json::from_str::<ProbabilityQualityBody>(&encoded).expect("deserialize full"),
        ProbabilityQualityBody::Full
    );
    assert!(serde_json::from_str::<ProbabilityQualityBody>(r#""estimated""#).is_err());
}

#[tokio::test]
async fn store_backed_request_uses_calendar_window_and_topological_order() {
    let state = test_state().await;
    let first = admit_task(&state, "First", json!({"duration_minutes": 15})).await;
    let second = admit_task(
        &state,
        "Second",
        json!({"duration_minutes": 20, "blocked_by": [first.clone()]}),
    )
    .await;
    store_calendar_window(&state, "2026-06-10T15:00:00Z", "2026-06-10T17:00:00Z").await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");

    let window = request.time_window.as_ref().expect("time window");
    assert_eq!(window.start, timestamp_minutes("2026-06-10T15:00:00Z"));
    assert_eq!(window.end, timestamp_minutes("2026-06-10T17:00:00Z"));
    assert_eq!(
        request
            .task_graph
            .as_ref()
            .expect("task graph")
            .topological_order,
        vec![first.clone(), second]
    );
    assert_eq!(request.mode, PlanningModeBody::FreshGeneration);
    assert!(request.rng_seed.is_some());
    assert_eq!(request.compute_budget.n_rollouts, 1_000);
    assert_eq!(request.compute_budget.top_k, 3);
    assert!(!request.strict_validation);
    assert_eq!(request.scoring_policy.utility_weight, 1.0);
    assert_eq!(request.scoring_policy.robustness_weight, 1.0);
    assert_eq!(request.scoring_policy.affect_margin_weight, 1.0);
    assert_eq!(request.scoring_policy.schedule_diversity_weight, 1.0);

    let kernel_request = PlanningRequest::from(request);
    assert_eq!(kernel_request.scoring_policy.utility_weight, 1.0);
    assert_eq!(kernel_request.scoring_policy.robustness_weight, 1.0);
    assert_eq!(kernel_request.scoring_policy.affect_margin_weight, 1.0);
    assert_eq!(kernel_request.scoring_policy.schedule_diversity_weight, 1.0);
    assert_eq!(kernel_request.n_rollouts, 1_000);
    assert_eq!(kernel_request.top_k, 3);
    assert!(!kernel_request.strict_validation);
    let first_task = kernel_request
        .task_graph
        .tasks
        .iter()
        .find(|task| task.id == first)
        .expect("first task");
    assert_eq!(first_task.duration, DurationModel::Fixed { seconds: 15 });
    assert!(first_task.correlation_groups.is_empty());
}

#[tokio::test]
async fn store_backed_request_forwards_duration_estimate_and_correlations() {
    let state = test_state().await;
    admit_task(
        &state,
        "Stochastic task",
        json!({
            "duration_estimate": {
                "type": "shifted_lognormal_p95",
                "min_seconds": 300,
                "mode_seconds": 900,
                "p95_seconds": 3600
            },
            "correlation_groups": [{
                "group": "shared-test-environment",
                "strength": 0.75
            }]
        }),
    )
    .await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    let kernel_request = PlanningRequest::from(request);
    let task = &kernel_request.task_graph.tasks[0];
    assert_eq!(
        task.duration,
        DurationModel::ShiftedLognormalP95 {
            min_seconds: 300,
            mode_seconds: 900,
            p95_seconds: 3600,
        }
    );
    assert_eq!(task.correlation_groups.len(), 1);
    assert_eq!(task.correlation_groups[0].group, "shared-test-environment");
    assert_eq!(task.correlation_groups[0].strength, 0.75);
}

#[tokio::test]
async fn store_backed_request_filters_precondition_blocked_and_invalid_tasks() {
    let state = test_state().await;
    admit_universe_state(
        &state,
        json!({
            "ticket.status": "ready"
        }),
    )
    .await;

    let no_precondition =
        admit_task(&state, "No precondition", json!({"duration_minutes": 10})).await;
    let satisfied = admit_task(
        &state,
        "Satisfied",
        json!({
            "duration_minutes": 10,
            "preconditions": {
                "target": "facts.ticket.status",
                "predicate": "equals",
                "expected": "ready"
            }
        }),
    )
    .await;
    let blocked = admit_task(
        &state,
        "Blocked",
        json!({
            "duration_minutes": 10,
            "preconditions": {
                "target": "facts.ticket.status",
                "predicate": "equals",
                "expected": "done"
            }
        }),
    )
    .await;
    let invalid = admit_task(
        &state,
        "Invalid",
        json!({
            "duration_minutes": 10,
            "preconditions": {
                "target": "not_a_collection.ticket.status",
                "predicate": "equals",
                "expected": "ready"
            }
        }),
    )
    .await;
    let unknown_absent = admit_task(
        &state,
        "Unknown absent",
        json!({
            "duration_minutes": 10,
            "preconditions": {
                "target": "facts.ticket.owner",
                "predicate": "absent"
            }
        }),
    )
    .await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    let planned_task_ids = request
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();

    assert!(planned_task_ids.contains(&no_precondition));
    assert!(planned_task_ids.contains(&satisfied));
    assert!(planned_task_ids.contains(&unknown_absent));
    assert!(!planned_task_ids.contains(&blocked));
    assert!(!planned_task_ids.contains(&invalid));

    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(json_request(
            "/planning/generate",
            json!({"schema_version": PLANNING_SCHEMA_VERSION}),
        ))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["blocked_tasks"][0]["task_id"], blocked);
    assert_eq!(
        body["blocked_tasks"][0]["precondition"]["target"],
        "facts.ticket.status"
    );
    assert_eq!(body["invalid_tasks"][0]["task_id"], invalid);
    assert!(body["invalid_tasks"][0]["error"]
        .as_str()
        .expect("invalid error")
        .contains("malformed precondition"));

    let diagnostic_codes = body["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .collect::<Vec<_>>();
    assert!(diagnostic_codes.contains(&"task_precondition_blocked"));
    assert!(diagnostic_codes.contains(&"task_precondition_invalid"));
}

#[tokio::test]
async fn store_backed_request_defaults_missing_model_to_fixed_independent() {
    let state = test_state().await;
    admit_task(&state, "Default duration task", json!({})).await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    assert!(request.tasks[0].duration_estimate.is_none());
    assert!(request.tasks[0].correlation_groups.is_empty());

    let kernel_request = PlanningRequest::from(request);
    let task = &kernel_request.task_graph.tasks[0];
    assert_eq!(task.duration, DurationModel::Fixed { seconds: 30 });
    assert!(task.correlation_groups.is_empty());
}

#[tokio::test]
async fn ranked_candidates_are_persisted_and_calendar_selects_rank_one() {
    let state = test_state().await;
    admit_task(
        &state,
        "First candidate task",
        json!({"duration_minutes": 10}),
    )
    .await;
    admit_task(
        &state,
        "Second candidate task",
        json!({"duration_minutes": 10}),
    )
    .await;
    store_calendar_window(&state, "2026-06-10T15:00:00Z", "2026-06-10T18:00:00Z").await;
    let app = ubu_orchestrator::build_router(state);

    let response = app
        .clone()
        .oneshot(json_request(
            "/planning/generate",
            json!({"schema_version": PLANNING_SCHEMA_VERSION}),
        ))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);
    let generated = json_body(response).await;

    assert_eq!(generated["schema_version"], PLANNING_SCHEMA_VERSION);
    assert_eq!(generated["selected_candidate"]["rank"], 1);
    assert!(generated["selected_candidate"]["display_probability"].is_number());
    assert!(generated["selected_candidate"]["probability_interval_low"].is_number());
    assert!(generated["selected_candidate"]["probability_interval_high"].is_number());
    assert!(generated["selected_candidate"]["robustness_score"].is_number());
    assert_eq!(
        generated["selected_candidate"]["probability_quality"],
        "full"
    );
    assert!(!generated["alternatives"]
        .as_array()
        .expect("alternatives")
        .is_empty());
    assert!(generated["alternatives"].as_array().unwrap().len() <= 15);
    assert_eq!(
        generated["plan"]["steps"],
        generated["selected_candidate"]["steps"]
    );
    let selected_total = generated["selected_candidate"]["score_summary"]["total_score"]
        .as_f64()
        .expect("selected total score");
    let mut saw_retained_non_finalist = false;
    for alternative in generated["alternatives"].as_array().unwrap() {
        let rank = alternative["rank"].as_u64().unwrap();
        assert!(rank > 1);
        assert!(alternative["candidate_role"].is_string());
        assert!(alternative["score_summary"]["total_score"].is_number());
        assert!(alternative["semi_legitimization_summary"]["result"].is_string());
        assert!(alternative["robustness_score"].is_number());
        if rank <= 3 {
            assert_eq!(alternative["probability_quality"], "full");
            assert!(alternative["display_probability"].is_number());
            assert!(alternative["probability_interval_low"].is_number());
            assert!(alternative["probability_interval_high"].is_number());
            assert!(
                selected_total
                    >= alternative["score_summary"]["total_score"]
                        .as_f64()
                        .expect("alternative total score")
            );
        } else {
            saw_retained_non_finalist = true;
            assert_eq!(alternative["probability_quality"], "not_estimated");
            assert!(alternative["display_probability"].is_null());
            assert!(alternative["probability_interval_low"].is_null());
            assert!(alternative["probability_interval_high"].is_null());
        }
    }
    assert!(saw_retained_non_finalist);
    let calendar_response = app
        .clone()
        .oneshot(get_request("/calendar/current"))
        .await
        .expect("calendar response");
    assert_eq!(calendar_response.status(), StatusCode::OK);
    let calendar = json_body(calendar_response).await;
    assert_eq!(
        calendar["selected_candidate"],
        generated["selected_candidate"]
    );
    assert_eq!(calendar["alternatives"], generated["alternatives"]);
    assert_eq!(calendar["steps"], calendar["selected_candidate"]["steps"]);
    assert_eq!(
        calendar["display_probability"],
        calendar["selected_candidate"]["display_probability"]
    );
    assert_eq!(
        calendar["probability_interval_low"],
        calendar["selected_candidate"]["probability_interval_low"]
    );
    assert_eq!(
        calendar["probability_interval_high"],
        calendar["selected_candidate"]["probability_interval_high"]
    );
    assert_eq!(
        calendar["robustness_score"],
        calendar["selected_candidate"]["robustness_score"]
    );
    assert_eq!(
        calendar["probability_quality"],
        calendar["selected_candidate"]["probability_quality"]
    );

    let next_action_response = app
        .oneshot(get_request(&format!(
            "/next-action?schema_version={NEXT_ACTION_SCHEMA_VERSION}"
        )))
        .await
        .expect("next action response");
    assert_eq!(next_action_response.status(), StatusCode::OK);
    let next_action = json_body(next_action_response).await;
    assert_eq!(
        next_action["recommendation"]["task_id"],
        calendar["selected_candidate"]["steps"][0]["task_id"]
    );
}

#[tokio::test]
async fn zero_rollouts_surface_not_estimated_without_fabricated_probability() {
    let state = test_state().await;
    admit_task(&state, "Interactive task", json!({"duration_minutes": 10})).await;
    let app = ubu_orchestrator::build_router(state);

    let response = app
        .clone()
        .oneshot(json_request(
            "/planning/generate",
            json!({
                "schema_version": PLANNING_SCHEMA_VERSION,
                "request": {
                    "schema_version": PLANNING_SCHEMA_VERSION,
                    "request_id": "interactive-rollout-skip",
                    "compute_budget": {"n_rollouts": 0, "top_k": 3},
                    "time_window": {"start": 0, "end": 100},
                    "tasks": [{
                        "id": "interactive-task",
                        "duration": 10,
                        "window": {"start": 0, "end": 100}
                    }]
                }
            }),
        ))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);
    let generated = json_body(response).await;
    assert_eq!(
        generated["selected_candidate"]["probability_quality"],
        "not_estimated"
    );
    assert!(generated["selected_candidate"]["display_probability"].is_null());
    assert!(generated["selected_candidate"]["probability_interval_low"].is_null());
    assert!(generated["selected_candidate"]["probability_interval_high"].is_null());

    let calendar = json_body(
        app.oneshot(get_request("/calendar/current"))
            .await
            .expect("calendar response"),
    )
    .await;
    assert_eq!(calendar["probability_quality"], "not_estimated");
    assert!(calendar["display_probability"].is_null());
    assert!(calendar["probability_interval_low"].is_null());
    assert!(calendar["probability_interval_high"].is_null());
}

#[tokio::test]
async fn store_backed_request_uses_affect_preferences_and_fresh_snapshot() {
    let state = test_state().await;
    admit_task(&state, "Plan with affect", json!({"duration_minutes": 15})).await;
    admit_preference(&state, "acceptable_energy_floor", json!("high")).await;
    admit_preference(&state, "tolerable_stress_ceiling", json!(6.0)).await;
    admit_preference(&state, "tolerable_intensity_ceiling", json!("moderate")).await;
    store_calendar_window(&state, "2026-06-10T15:00:00Z", "2026-06-10T16:00:00Z").await;
    admit_snapshot(
        &state,
        "2026-06-10T15:00:00Z",
        json!({
            "energy": 8.0,
            "stress": 4.0,
            "mood_intensity": 5.0
        }),
    )
    .await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    let profile = request.affect_profile.expect("affect profile");
    let observation = request.affect_observation.expect("affect observation");

    assert_eq!(profile.mode, AffectLegitimizationModeBody::Enforce);
    assert_eq!(profile.dimensions["energy"].location, 7.0);
    assert_eq!(profile.dimensions["stress"].location, 6.0);
    assert_eq!(profile.dimensions["mood_intensity"].location, 5.0);
    assert_eq!(
        observation.dimensions["energy"].source_kind,
        "live_observation"
    );
    assert!(request.affect_warning.is_none());
    let task = serde_json::to_value(&request.tasks[0]).expect("task json");
    assert!(task.get("affect_required").is_none());
    assert!(task.get("affect_current").is_none());
}

#[tokio::test]
async fn missing_affect_snapshot_uses_bootstrap_default_observation_in_warn_only() {
    let state = test_state().await;
    admit_task(
        &state,
        "Plan without snapshot",
        json!({"duration_minutes": 15}),
    )
    .await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    let profile = request.affect_profile.expect("affect profile");
    let observation = request.affect_observation.expect("affect observation");

    assert_eq!(profile.mode, AffectLegitimizationModeBody::WarnOnly);
    assert_eq!(profile.dimensions["energy"].location, 4.0);
    assert_eq!(profile.dimensions["stress"].location, 7.0);
    assert_eq!(profile.dimensions["mood_intensity"].location, 8.0);
    assert_eq!(
        observation.dimensions["energy"].source_kind,
        "bootstrap_default_profile"
    );
    assert!(request
        .affect_warning
        .as_deref()
        .expect("affect warning")
        .contains("missing affect observation"));
}

#[tokio::test]
async fn stale_affect_snapshot_is_not_presented_as_current() {
    let state = test_state().await;
    admit_task(
        &state,
        "Plan with stale affect",
        json!({"duration_minutes": 15}),
    )
    .await;
    admit_preference(&state, "affect_freshness_seconds", json!(60)).await;
    store_calendar_window(&state, "2026-06-10T15:00:00Z", "2026-06-10T16:00:00Z").await;
    admit_snapshot(
        &state,
        "2026-06-10T14:00:00Z",
        json!({
            "energy": 1.0,
            "stress": 10.0,
            "mood_intensity": 10.0
        }),
    )
    .await;

    let request = planning_service::build_request_from_store(&state)
        .await
        .expect("request");
    let profile = request.affect_profile.expect("affect profile");
    let observation = request.affect_observation.expect("affect observation");

    assert_eq!(profile.mode, AffectLegitimizationModeBody::WarnOnly);
    assert_eq!(
        observation.dimensions["energy"].source_kind,
        "bootstrap_default_profile"
    );
    assert_eq!(
        observation.dimensions["energy"].observed_at,
        timestamp_minutes("2026-06-10T15:00:00Z")
    );
    assert!(request
        .affect_warning
        .as_deref()
        .expect("affect warning")
        .contains("stale affect observation"));
}

#[tokio::test]
async fn generate_persists_canonical_timed_plan_and_current_calendar_serves_steps() {
    let state = test_state().await;
    admit_task(&state, "Plan me", json!({"duration_minutes": 10})).await;
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", json!({})))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    let row =
        sqlx::query("SELECT status, payload_json FROM plans ORDER BY created_at DESC LIMIT 1")
            .fetch_one(state.inner().store.pool())
            .await
            .expect("plan row");
    let status: String = row.try_get("status").expect("status");
    let payload_json: String = row.try_get("payload_json").expect("payload");
    let payload: Value = serde_json::from_str(&payload_json).expect("plan json");
    assert_eq!(status, "admitted");
    assert!(payload.get("id").and_then(Value::as_str).is_some());
    assert_eq!(payload["status"], "admitted");
    assert!(payload.get("steps").and_then(Value::as_array).is_some());
    assert!(payload["steps"][0].get("start").is_some());
    assert!(payload["steps"][0].get("end").is_some());
    assert!(payload.get("tasks").is_none());
    assert_eq!(payload["legitimization"]["mode"], "warn_only");
    assert!(payload["legitimization"]["dimensions"]["energy"]
        .get("satisfaction")
        .is_some());
    assert!(payload["legitimization"]
        .get("stale_affect_warning")
        .and_then(Value::as_str)
        .expect("affect warning")
        .contains("missing affect observation"));
    assert!(payload["risk_report"]["findings"].is_array());
    assert_eq!(
        payload["human_complete_plan_quality"]["plan_ref"],
        payload["id"]
    );
    assert!(payload["human_complete_plan_quality"]["feedback_latency"].is_u64());

    let calendar_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/calendar/current")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("calendar response");
    assert_eq!(calendar_response.status(), StatusCode::OK);
    let body = json_body(calendar_response).await;
    assert!(body["steps"].as_array().expect("steps").len() == 1);
    assert_eq!(body["legitimization"]["mode"], "warn_only");
    assert!(body["legitimization"]["dimensions"]["energy"]
        .get("satisfaction")
        .is_some());
    assert!(body["risk_report"]["findings"].is_array());
    assert!(body["human_complete_plan_quality"]["revision_suggestions"].is_array());

    let next_action = app
        .oneshot(get_request(&format!(
            "/next-action?schema_version={NEXT_ACTION_SCHEMA_VERSION}"
        )))
        .await
        .expect("next-action response");
    let next_action = json_body(next_action).await;
    assert!(next_action["risk_report"]["findings"].is_array());
    assert!(next_action["human_complete_plan_quality"]["plan_ref"].is_string());
}

#[tokio::test]
async fn blocking_deadline_risk_marks_calendar_stale_and_raises_recalculation() {
    let state = test_state().await;
    let task_id = admit_task(
        &state,
        "Impossible deadline",
        json!({"duration_minutes": 10, "due_at": 0}),
    )
    .await;
    let app = ubu_orchestrator::build_router(state.clone());

    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", json!({})))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["risk_report"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|finding| finding["category"] == "deadline_risk"
            && finding["blocking"] == true
            && finding["subject_ref"] == task_id));

    let calendar = app
        .oneshot(get_request("/calendar/current"))
        .await
        .expect("calendar response");
    let calendar = json_body(calendar).await;
    assert_eq!(calendar["stale"], true);

    let trigger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM logs WHERE event_type = 'recalculation_requested'",
    )
    .fetch_one(state.inner().store.pool())
    .await
    .expect("trigger count");
    assert_eq!(trigger_count, 1);
}

#[tokio::test]
async fn recalculation_supersedes_prior_plan_and_preserves_user_override_placement() {
    let state = test_state().await;
    let frozen = admit_task(&state, "Frozen", json!({"duration_minutes": 30})).await;
    let remaining = admit_task(
        &state,
        "Remaining",
        json!({"duration_minutes": 30, "blocked_by": [frozen.clone()]}),
    )
    .await;
    let app = ubu_orchestrator::build_router(state.clone());
    let response = app
        .clone()
        .oneshot(json_request("/planning/generate", json!({})))
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    append_user_override_log(&state, &frozen).await;
    let prior = planning_service::latest_admitted_plan(&state)
        .await
        .expect("prior query")
        .expect("prior plan");
    let frozen_before = prior
        .steps
        .iter()
        .find(|step| step.task_id == frozen)
        .expect("frozen step")
        .clone();

    let response = app
        .oneshot(json_request(
            "/planning/recalculate",
            json!({
                "schema_version": RECALCULATION_SCHEMA_VERSION,
                "triggered_at": "2026-06-10T18:00:00Z",
                "trigger_type": "user_override",
                "objects": [{"id": frozen, "object_type": "Task"}]
            }),
        ))
        .await
        .expect("recalculate response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["prior_plan_id"], prior.id);
    assert_eq!(body["plan"]["supersedes_plan_id"], prior.id);

    let new_frozen = body["plan"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["task_id"] == frozen_before.task_id)
        .expect("preserved frozen step");
    assert_eq!(new_frozen["start"], frozen_before.start);
    assert_eq!(new_frozen["end"], frozen_before.end);

    let new_remaining = body["plan"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["task_id"] == remaining)
        .expect("remaining step");
    assert!(
        new_remaining["start"].as_u64().expect("remaining start") >= frozen_before.end,
        "remaining work must not be placed before the frozen override"
    );

    let superseded: String = sqlx::query("SELECT status FROM plans WHERE id = ?")
        .bind(&prior.id)
        .fetch_one(state.inner().store.pool())
        .await
        .expect("prior row")
        .try_get("status")
        .expect("status");
    assert_eq!(superseded, "superseded");
}

#[tokio::test]
async fn planning_generate_rejects_unknown_schema_version() {
    let state = test_state().await;
    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(json_request(
            "/planning/generate",
            json!({"schema_version": "unknown"}),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "unknown_schema_version");
}

#[tokio::test]
async fn planning_generate_rejects_invalid_duration_estimate_with_diagnostic() {
    let state = test_state().await;
    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(json_request(
            "/planning/generate",
            json!({
                "schema_version": PLANNING_SCHEMA_VERSION,
                "request": {
                    "schema_version": PLANNING_SCHEMA_VERSION,
                    "request_id": "invalid-duration-estimate",
                    "tasks": [{
                        "id": "task-1",
                        "duration": 30,
                        "duration_estimate": {
                            "type": "shifted_lognormal_p95",
                            "min_seconds": 900,
                            "mode_seconds": 300,
                            "p95_seconds": 3600
                        }
                    }]
                }
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["diagnostics"][0]["code"], "invalid_duration_estimate");
    assert!(body["diagnostics"][0]["message"]
        .as_str()
        .expect("diagnostic message")
        .contains("min_seconds < mode_seconds < p95_seconds"));
}

#[tokio::test]
async fn planning_generate_carries_stochastic_duration_to_kernel() {
    let state = test_state().await;
    let app = ubu_orchestrator::build_router(state);
    let response = app
        .oneshot(json_request(
            "/planning/generate",
            json!({
                "schema_version": PLANNING_SCHEMA_VERSION,
                "request": {
                    "schema_version": PLANNING_SCHEMA_VERSION,
                    "request_id": "stochastic-duration-end-to-end",
                    "compute_budget": {"n_rollouts": 0, "top_k": 1},
                    "time_window": {"start": 0, "end": 10_000},
                    "tasks": [{
                        "id": "task-1",
                        "duration": 30,
                        "duration_estimate": {
                            "type": "shifted_lognormal_p95",
                            "min_seconds": 300,
                            "mode_seconds": 900,
                            "p95_seconds": 3600
                        },
                        "correlation_groups": [{
                            "group": "shared-test-environment",
                            "strength": 0.75
                        }]
                    }]
                }
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schema_version"], PLANNING_SCHEMA_VERSION);
    let step = &body["plan"]["steps"][0];
    assert_eq!(
        step["end"].as_u64().expect("end") - step["start"].as_u64().expect("start"),
        900
    );
}

async fn test_state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state")
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

    queries::admit_object(
        state.inner().store.pool(),
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

async fn admit_universe_state(state: &AppState, facts: Value) -> String {
    let id = UbuId::new(ObjectType::UniverseState).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    queries::admit_object(
        state.inner().store.pool(),
        NewObjectRecord {
            id: id.clone(),
            object_type: ObjectType::UniverseState.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "test".to_owned(),
            payload: json!({
                "id": id,
                "captured_at": now,
                "facts": facts,
                "source_summary": "test UniverseState",
                "provenance": {
                    "created_at": now,
                    "authority_source": "user"
                }
            }),
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .expect("UniverseState admitted");
    id
}

async fn admit_preference(state: &AppState, name: &str, value: Value) -> String {
    let id = UbuId::new(ObjectType::Preference).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    queries::admit_object(
        state.inner().store.pool(),
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
    queries::admit_object(
        state.inner().store.pool(),
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
    queries::append_log_entry(
        state.inner().store.pool(),
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

fn timestamp_minutes(value: &str) -> u64 {
    UbuTimestamp::parse(value)
        .expect("timestamp")
        .inner()
        .unix_timestamp() as u64
        / 60
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
