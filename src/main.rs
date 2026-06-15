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
