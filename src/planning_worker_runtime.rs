// Compiled only into the executable. No orchestrator test can reach this probe
// or instantiate the owned worker transport through application state defaults.
use std::time::Duration;
use ubu_orchestrator::adapters::planning_worker::PlanningWorkerResult;
use ubu_planning_core::PlanningRequest;
use ubu_planning_worker::{
    stage1::{plan_stage1, LocalStageTransport, Stage1Strategy},
    LocalEnvironment,
};

pub fn plan(request: PlanningRequest) -> PlanningWorkerResult {
    // Selection already existed in P1B-74. The kernel now retains its source,
    // actual command spelling and bounded probe outcome as well.
    let environment = LocalEnvironment::detect();
    let python = environment.interpreter.clone();
    // Called only for the operator's explicit enabled=true. That opt-in is the
    // approved local compute-budget justification. Kernel eligibility and the
    // owned session both retain the shared nonblocking build/compute lock.
    let strategy = Stage1Strategy::new(
        true,
        environment.clone(),
        true,
        LocalStageTransport::new(python, Duration::from_secs(30)),
    );
    let response = plan_stage1(request, &strategy);
    PlanningWorkerResult {
        response,
        fallback: strategy.fallback_reason(),
        environment,
    }
    // Drop reaps the session and releases compute before persistence or a build.
}
