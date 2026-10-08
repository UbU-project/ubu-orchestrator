// Compiled only into the executable. No orchestrator test can reach this probe
// or instantiate the owned worker transport through application state defaults.
use std::time::Duration;
use ubu_planning_core::{PlanningRequest, PlanningResponse};
use ubu_planning_worker::{
    stage1::{plan_stage1, LocalStageTransport, Stage1FallbackReason, Stage1Strategy},
    LocalEnvironment,
};

pub fn plan(request: PlanningRequest) -> (PlanningResponse, Option<Stage1FallbackReason>) {
    let python = std::env::var("UBU_PLANNING_WORKER_PYTHON").unwrap_or_else(|_| "python3".into());
    let environment = LocalEnvironment::detect_with_python(&python);
    // Called only for the operator's explicit enabled=true. That opt-in is the
    // approved local compute-budget justification. Kernel eligibility and the
    // owned session both retain the shared nonblocking build/compute lock.
    let strategy = Stage1Strategy::new(
        true,
        environment,
        true,
        LocalStageTransport::new(python, Duration::from_secs(30)),
    );
    let response = plan_stage1(request, &strategy);
    (response, strategy.fallback_reason())
    // Drop reaps the session and releases compute before persistence or a build.
}
