//! `lifeagent-server` entry point: configuration from the environment (`config.rs`), logs to
//! stdout for Cloud Logging, graceful shutdown on SIGTERM (what Cloud Run sends).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use lifeagent_core::clock::SystemClock;
use lifeagent_server::config::ServerConfig;
use lifeagent_server::{router, AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env()?;
    let bind = config.bind;
    let state = AppState::new(config, Arc::new(SystemClock)).await?;
    state.spawn_background();

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "listening");
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
