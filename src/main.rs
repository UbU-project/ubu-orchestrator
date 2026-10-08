#[cfg(not(test))]
mod ollama_transport;
#[cfg(not(test))]
mod planning_worker_runtime;

use std::net::SocketAddr;

use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::router::build_router;
use ubu_orchestrator::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ubu_orchestrator::tracing::init_tracing();

    let config = ServerConfig::from_env();
    let addr = config.bind_addr();
    assert_loopback(addr);

    let state = AppState::new(config).await?;
    #[cfg(not(test))]
    let state = state.with_advisory_transport_factory(std::sync::Arc::new(|endpoint| {
        std::sync::Arc::new(ollama_transport::OllamaTransport::new(endpoint))
    }));
    #[cfg(not(test))]
    let state = state.with_planning_worker_factory(std::sync::Arc::new(planning_worker_runtime::plan));
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(%addr, "ubu-orchestrator listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn assert_loopback(addr: SocketAddr) {
    assert!(
        addr.ip().is_loopback(),
        "Phase 1 HTTP server must bind to loopback only"
    );
}
