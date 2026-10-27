use anyhow::Context;
use mm_server::{app, AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let port: u16 = match std::env::var("MM_PORT") {
        Ok(p) => p.parse().context("MM_PORT must be a port number")?,
        Err(_) => 8080,
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("mm-server listening on http://{addr}");
    axum::serve(listener, app(AppState::open_default()?)).await?;
    Ok(())
}
