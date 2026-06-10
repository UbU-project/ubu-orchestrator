#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanCompleteReport {
    pub completed_tasks: usize,
    pub notes: Vec<String>,
}
