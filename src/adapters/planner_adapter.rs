use ubu_planning_core::{PlanningRequest, PlanningResponse, RepairRequest, RepairResponse};

pub trait PlannerAdapter {
    fn plan(&self, request: PlanningRequest) -> PlanningResponse;
    fn repair(&self, request: RepairRequest) -> RepairResponse;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuPlannerAdapter;

impl PlannerAdapter for CpuPlannerAdapter {
    fn plan(&self, request: PlanningRequest) -> PlanningResponse {
        ubu_planning_core::plan(request, &ubu_planning_cpu::CpuStrategy)
    }

    fn repair(&self, request: RepairRequest) -> RepairResponse {
        ubu_planning_core::repair(request, &ubu_planning_cpu::CpuStrategy)
    }
}
