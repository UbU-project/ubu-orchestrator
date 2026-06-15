use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::main]
async fn main() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .expect("state");
    let _app = ubu_orchestrator::build_router(state);
    println!("fixture loop router initialized");
}
