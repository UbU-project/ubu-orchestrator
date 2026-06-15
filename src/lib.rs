pub mod adapters;
pub mod api;
pub mod config;
pub mod errors;
pub mod openapi;
pub mod reports;
pub mod router;
pub mod services;
pub mod state;
pub mod tracing;

pub use router::build_router;

#[cfg(test)]
mod integration {
    use crate::api::github::ImportFixtureRequest;
    use crate::api::planning::GeneratePlanningRequest;
    use crate::config::ServerConfig;
    use crate::services::{import_service, next_action_service, planning_service};
    use crate::state::AppState;

    async fn test_state() -> AppState {
        AppState::in_memory(ServerConfig::from_env())
            .await
            .expect("in-memory store initializes")
    }

    #[tokio::test]
    async fn import_then_plan_then_next_action() {
        let state = test_state().await;

        let import_resp = import_service::import_fixture(
            state.clone(),
            ImportFixtureRequest {
                fixture_path: "fixtures/fixture-loop/github-small.json".to_owned(),
            },
        )
        .await
        .expect("fixture import succeeds");

        assert_eq!(
            import_resp.imported,
            import_resp.admitted_to_store,
            "all candidates admitted"
        );
        assert!(
            !import_resp.candidates.is_empty(),
            "at least one candidate"
        );

        let plan_resp = planning_service::generate(
            state.clone(),
            GeneratePlanningRequest { request: None },
        )
        .await
        .expect("plan generation succeeds");

        assert!(plan_resp.plan.is_some(), "plan was generated");

        let next = next_action_service::get_next_action(state.clone())
            .await
            .expect("next action available after planning");

        assert!(!next.task_id.is_empty(), "next action has a task_id");
        assert!(next.readiness, "task is reported ready");
        assert!(next.end > next.start, "scheduled window is valid");
    }

    #[tokio::test]
    async fn admitted_state_survives_across_separate_requests() {
        let state = test_state().await;

        import_service::import_fixture(
            state.clone(),
            ImportFixtureRequest {
                fixture_path: "fixtures/fixture-loop/github-small.json".to_owned(),
            },
        )
        .await
        .expect("import on first request");

        planning_service::generate(
            state.clone(),
            GeneratePlanningRequest { request: None },
        )
        .await
        .expect("plan on second request");

        let next = next_action_service::get_next_action(state.clone())
            .await
            .expect("next action on third request");

        assert!(!next.task_id.is_empty());
    }
}
