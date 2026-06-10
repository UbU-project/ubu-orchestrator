use ubu_planning_core::{PlanningRequest, PlanningResponse};

pub trait PlannerAdapter {
    fn plan(&self, request: PlanningRequest) -> PlanningResponse;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuPlannerAdapter;

impl PlannerAdapter for CpuPlannerAdapter {
    fn plan(&self, request: PlanningRequest) -> PlanningResponse {
        ubu_planning_core::plan(request, &ubu_planning_cpu::CpuStrategy::default())
    }
}
