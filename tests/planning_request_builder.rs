#[test]
fn planning_request_builder_scaffold_exists() {
    assert!(std::path::Path::new("src/services/planning_service.rs").exists());
}

#[test]
fn scoring_policy_is_forwarded_without_mutation() {
    let body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "scoring-policy-test",
            "scoring_policy": {
                "utility_weight": 4.0,
                "robustness_weight": 3.0,
                "affect_margin_weight": 2.0,
                "schedule_diversity_weight": 1.0
            },
            "tasks": [{"id": "task-1", "duration": 10}]
        }))
        .expect("planning request body");

    let request = ubu_planning_core::PlanningRequest::from(body);
    assert_eq!(request.scoring_policy.utility_weight, 4.0);
    assert_eq!(request.scoring_policy.robustness_weight, 3.0);
    assert_eq!(request.scoring_policy.affect_margin_weight, 2.0);
    assert_eq!(request.scoring_policy.schedule_diversity_weight, 1.0);
}

#[test]
fn rollout_budget_defaults_and_caps_are_forwarded_to_kernel() {
    let default_body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "rollout-defaults",
            "tasks": [{"id": "task-1", "duration": 10}]
        }))
        .expect("planning request body");
    let default_request = ubu_planning_core::PlanningRequest::from(default_body);
    assert_eq!(default_request.n_rollouts, 1_000);
    assert_eq!(default_request.top_k, 3);
    assert!(!default_request.strict_validation);

    let capped_body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "rollout-caps",
            "compute_budget": {"n_rollouts": 9_000, "top_k": 20},
            "strict_validation": true,
            "tasks": [{"id": "task-1", "duration": 10}]
        }))
        .expect("planning request body");
    let capped_request = ubu_planning_core::PlanningRequest::from(capped_body);
    assert_eq!(capped_request.n_rollouts, 5_000);
    assert_eq!(capped_request.top_k, 8);
    assert!(capped_request.strict_validation);
}

#[test]
fn zero_rollouts_preserves_interactive_skip_mode() {
    let body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "rollout-skip",
            "compute_budget": {"n_rollouts": 0, "top_k": 3},
            "tasks": [{"id": "task-1", "duration": 10}]
        }))
        .expect("planning request body");

    let request = ubu_planning_core::PlanningRequest::from(body);
    assert_eq!(request.n_rollouts, 0);
}

#[test]
fn stochastic_duration_and_correlations_are_forwarded_to_kernel() {
    let body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "stochastic-duration",
            "tasks": [{
                "id": "task-1",
                "duration": 10,
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
        }))
        .expect("planning request body");

    let request = ubu_planning_core::PlanningRequest::from(body);
    let task = &request.task_graph.tasks[0];
    assert_eq!(
        task.duration,
        ubu_planning_core::DurationModel::ShiftedLognormalP95 {
            min_seconds: 300,
            mode_seconds: 900,
            p95_seconds: 3600,
        }
    );
    assert_eq!(task.correlation_groups.len(), 1);
    assert_eq!(task.correlation_groups[0].group, "shared-test-environment");
    assert_eq!(task.correlation_groups[0].strength, 0.75);
}

#[test]
fn scalar_duration_remains_the_fixed_default() {
    let body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "fixed-duration-default",
            "tasks": [{"id": "task-1", "duration": 42}]
        }))
        .expect("planning request body");

    let request = ubu_planning_core::PlanningRequest::from(body);
    assert_eq!(
        request.task_graph.tasks[0].duration,
        ubu_planning_core::DurationModel::Fixed { seconds: 42 }
    );
    assert!(request.task_graph.tasks[0].correlation_groups.is_empty());

    let explicit_body: ubu_orchestrator::api::planning::PlanningRequestBody =
        serde_json::from_value(serde_json::json!({
            "request_id": "explicit-fixed-duration",
            "tasks": [{
                "id": "task-1",
                "duration": 42,
                "duration_estimate": {"type": "fixed", "seconds": 600}
            }]
        }))
        .expect("planning request body");
    let explicit_request = ubu_planning_core::PlanningRequest::from(explicit_body);
    assert_eq!(
        explicit_request.task_graph.tasks[0].duration,
        ubu_planning_core::DurationModel::Fixed { seconds: 600 }
    );
}
