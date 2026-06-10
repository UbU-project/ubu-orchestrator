use crate::api::next_action::NextActionResponse;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn get_next_action(state: AppState) -> Result<NextActionResponse> {
    let memory = state.inner().memory.lock().await;
    memory
        .next_action
        .clone()
        .ok_or_else(|| AppError::NotFound("no next action available".to_owned()))
}
