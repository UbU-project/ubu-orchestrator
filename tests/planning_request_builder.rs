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
