use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::state::AppState;

#[tokio::main]
async fn main() {
    let _app = ubu_orchestrator::build_router(AppState::new(ServerConfig::from_env()));
    println!("fixture loop router initialized");
}
