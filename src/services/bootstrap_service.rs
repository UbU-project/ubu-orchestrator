use crate::api::bootstrap::{
    BootstrapAnswerRequest, BootstrapAnswerResponse, BootstrapStartResponse,
};
use crate::errors::Result;
use crate::state::AppState;

pub async fn start(state: AppState) -> Result<BootstrapStartResponse> {
    let mut memory = state.inner().memory.lock().await;
    memory.bootstrap_started = true;
    Ok(BootstrapStartResponse {
        started: true,
        next_prompt: "import_github_fixture".to_owned(),
    })
}

pub async fn answer(
    state: AppState,
    request: BootstrapAnswerRequest,
) -> Result<BootstrapAnswerResponse> {
    let mut memory = state.inner().memory.lock().await;
    memory.bootstrap_answers.push(request.answer);
    Ok(BootstrapAnswerResponse {
        accepted: true,
        answer_count: memory.bootstrap_answers.len(),
    })
}
