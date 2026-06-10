use crate::api::github::ImportedCandidate;
use crate::errors::Result;

pub trait StoreAdapter {
    fn admit_candidates(&self, candidates: &[ImportedCandidate]) -> Result<usize>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InMemoryStoreAdapter;

impl StoreAdapter for InMemoryStoreAdapter {
    fn admit_candidates(&self, candidates: &[ImportedCandidate]) -> Result<usize> {
        Ok(candidates.len())
    }
}
