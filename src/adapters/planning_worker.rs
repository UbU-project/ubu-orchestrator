//! Policy and response plumbing only. The executable supplies the kernel-owned
//! transport; library and orchestrator tests never probe or spawn an interpreter.
use super::planner_adapter::{CpuPlannerAdapter, PlannerAdapter};
use crate::{api::planning::DiagnosticBody, config::PlannerStrategyChoice, state::AppState};
use ubu_planning_core::{PlanningRequest, PlanningResponse};
use ubu_planning_worker::{
    stage1::{CertificationDifference, CertificationField, Stage1FallbackReason},
    InterpreterSource, LocalEnvironment,
};

/// Internal executable seam, never an API field. Tests inject only held facts.
pub struct PlanningWorkerResult {
    pub response: PlanningResponse,
    pub fallback: Option<Stage1FallbackReason>,
    pub environment: LocalEnvironment,
    pub certification_difference: Option<CertificationDifference>,
}
pub type PlanningWorkerFactory = dyn Fn(PlanningRequest) -> PlanningWorkerResult + Send + Sync;

pub fn interpreter_source_code(source: InterpreterSource) -> &'static str {
    match source {
        InterpreterSource::EnvironmentVariable => "planning_worker_python_environment_variable",
        InterpreterSource::Python3Fallback => "planning_worker_python3_fallback",
    }
}
pub fn certification_field_code(field: CertificationField) -> &'static str {
    match field {
        CertificationField::TaskIndex => "planning_gpu_fallback_certification_failed_task_index",
        CertificationField::SlotMask => "planning_gpu_fallback_certification_failed_slot_mask",
        CertificationField::StartTimeOffsets => {
            "planning_gpu_fallback_certification_failed_start_time_offsets"
        }
        CertificationField::DurationSamples => {
            "planning_gpu_fallback_certification_failed_duration_samples"
        }
        CertificationField::PieceIndex => "planning_gpu_fallback_certification_failed_piece_index",
        CertificationField::PieceCount => "planning_gpu_fallback_certification_failed_piece_count",
        CertificationField::ValidityMask => {
            "planning_gpu_fallback_certification_failed_validity_mask"
        }
        CertificationField::DependencySlack => {
            "planning_gpu_fallback_certification_failed_dependency_slack"
        }
        CertificationField::DependencyFeasibility => {
            "planning_gpu_fallback_certification_failed_dependency_feasibility"
        }
        CertificationField::HardConstraintFeasibility => {
            "planning_gpu_fallback_certification_failed_hard_constraint_feasibility"
        }
        CertificationField::RejectionCodes => {
            "planning_gpu_fallback_certification_failed_rejection_codes"
        }
        CertificationField::Omissions => "planning_gpu_fallback_certification_failed_omissions",
        CertificationField::Failure => "planning_gpu_fallback_certification_failed_failure",
    }
}
fn certification_diagnostic(difference: &CertificationDifference) -> DiagnosticBody {
    let mut message = difference.public_metadata();
    // This message reaches the existing PRIVATE screen. Debug/Display elsewhere
    // contain metadata alone, and the public projector selects metadata only.
    message
        .as_object_mut()
        .unwrap()
        .extend(difference.private_values().as_object().unwrap().clone());
    DiagnosticBody {
        code: certification_field_code(difference.field).into(),
        message: message.to_string(),
    }
}
fn environment_diagnostic(environment: &LocalEnvironment) -> DiagnosticBody {
    DiagnosticBody {
        code: interpreter_source_code(environment.interpreter_source).into(),
        // Messages are private in the live driver; JSON escaping bounds control text.
        message: format!(
            "Worker environment probe: interpreter={:?}; source={}; PyTorch version={:?}",
            environment.interpreter,
            environment.interpreter_source.as_str(),
            environment.torch_version
        ),
    }
}

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
            Self::Kernel(InterpreterStartFailed) => {
                "planning_gpu_fallback_interpreter_start_failed"
            }
            Self::Kernel(ModuleRootUnavailable) => "planning_gpu_fallback_module_root_unavailable",
            Self::Kernel(ModulePackageUnavailable) => {
                "planning_gpu_fallback_module_package_unavailable"
            }
            Self::Kernel(ProbeBudgetInvalid) => "planning_gpu_fallback_probe_budget_invalid",
            Self::Kernel(ProbeTimedOut) => "planning_gpu_fallback_probe_timed_out",
            Self::Kernel(ProbeFailed) => "planning_gpu_fallback_probe_failed",
            Self::Kernel(TorchVersionMismatch) => "planning_gpu_fallback_torch_version_mismatch",
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
) -> (PlanningResponse, Vec<DiagnosticBody>) {
    let cpu = CpuPlannerAdapter {
        strategy: state.inner().planner_strategy,
    };
    if !enabled {
        return (cpu.plan(request), Vec::new());
    }
    // stage1-atomic-v1 certifies CpuStrategy, never ChunkedSweep. Changing the
    // selected strategy to make the switch work would change the operator's Plan.
    if state.inner().planner_strategy != PlannerStrategyChoice::Greedy {
        return (
            cpu.plan(request),
            WorkerFallback::UnsupportedStrategy.diagnostics().to_vec(),
        );
    }
    let Some(factory) = state.planning_worker_factory() else {
        return (
            cpu.plan(request),
            WorkerFallback::TransportUnavailable.diagnostics().to_vec(),
        );
    };
    let result = factory(request);
    let mut diagnostics = vec![environment_diagnostic(&result.environment)];
    if let Some(reason) = result.fallback {
        diagnostics.extend(WorkerFallback::Kernel(reason).diagnostics());
        if reason == Stage1FallbackReason::CertificationFailed {
            if let Some(difference) = &result.certification_difference {
                diagnostics.push(certification_diagnostic(difference));
            }
        }
    }
    (result.response, diagnostics)
}
