//! Policy and response plumbing only. The executable supplies the kernel-owned
//! transport; library and orchestrator tests never probe or spawn an interpreter.
use super::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::{api::planning::DiagnosticBody, config::PlannerStrategyChoice, state::AppState};
use ubu_planning_core::{PlanningRequest, PlanningResponse};
use ubu_planning_worker::stage1::Stage1FallbackReason;

pub type PlanningWorkerFactory =
    dyn Fn(PlanningRequest) -> (PlanningResponse, Option<Stage1FallbackReason>) + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFallback {
    Kernel(Stage1FallbackReason),
    UnsupportedStrategy,
    TransportUnavailable,
}
impl WorkerFallback {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Kernel(reason) => reason.as_str(),
            Self::UnsupportedStrategy => "unsupported_strategy",
            Self::TransportUnavailable => "transport_unavailable",
        }
    }
    pub fn diagnostic_code(self) -> &'static str {
        use Stage1FallbackReason::*;
        // Exhaustive mapping: adding a kernel reason requires a public code.
        match self {
            Self::Kernel(PolicyDisabled) => "planning_gpu_fallback_policy_disabled",
            Self::Kernel(BudgetUnjustified) => "planning_gpu_fallback_budget_unjustified",
            Self::Kernel(PythonUnavailable) => "planning_gpu_fallback_python_unavailable",
            Self::Kernel(StageUnimplemented) => "planning_gpu_fallback_stage_unimplemented",
            Self::Kernel(TorchUnavailable) => "planning_gpu_fallback_torch_unavailable",
            Self::Kernel(ComputeLockUnavailable) => {
                "planning_gpu_fallback_compute_lock_unavailable"
            }
            Self::Kernel(InputUnsupported) => "planning_gpu_fallback_input_unsupported",
            Self::Kernel(TransportFailed) => "planning_gpu_fallback_transport_failed",
            Self::Kernel(ReplyMismatch) => "planning_gpu_fallback_reply_mismatch",
            Self::Kernel(CertificationFailed) => "planning_gpu_fallback_certification_failed",
            Self::UnsupportedStrategy => "planning_gpu_fallback_unsupported_strategy",
            Self::TransportUnavailable => "planning_gpu_fallback_transport_unavailable",
        }
    }
    pub fn diagnostics(self) -> [DiagnosticBody; 2] {
        [DiagnosticBody {code:"planning_gpu_unavailable".into(),message:format!("CPU reference used: {}",self.reason())},
         DiagnosticBody {code:self.diagnostic_code().into(),message:"The configured CPU strategy retained the Plan; the worker fallback reason is recorded in this code.".into()}]
    }
}

pub fn plan(
    state: &AppState,
    request: PlanningRequest,
    enabled: bool,
) -> (PlanningResponse, Option<WorkerFallback>) {
    let cpu = CpuPlannerAdapter {
        strategy: state.inner().planner_strategy,
    };
    if !enabled {
        return (cpu.plan(request), None);
    }
    // stage1-atomic-v1 certifies CpuStrategy, never ChunkedSweep. Changing the
    // selected strategy to make the switch work would change the operator's Plan.
    if state.inner().planner_strategy != PlannerStrategyChoice::Greedy {
        return (cpu.plan(request), Some(WorkerFallback::UnsupportedStrategy));
    }
    let Some(factory) = state.planning_worker_factory() else {
        return (
            cpu.plan(request),
            Some(WorkerFallback::TransportUnavailable),
        );
    };
    let (response, reason) = factory(request);
    (response, reason.map(WorkerFallback::Kernel))
}
