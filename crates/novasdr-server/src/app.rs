use crate::{shutdown, state, ws};
use anyhow::Context;
use axum::{routing::get, Router};
use std::{net::SocketAddr, sync::Arc};
use tower_http::{compression::CompressionLayer, services::ServeDir};

pub fn router(state: Arc<state::AppState>) -> Router {
    let html_root = state.html_root.clone();

    Router::new()
        .route("/server-info.json", get(state::server_info))
        .route("/receivers.json", get(state::receivers_info))
        .route("/audio", get(ws::audio::upgrade))
        .route("/waterfall", get(ws::waterfall::upgrade))
        .route("/events", get(ws::events::upgrade))
        .route("/chat", get(ws::chat::upgrade))
        .nest_service(
            "/",
            ServeDir::new(html_root).append_index_html_on_directories(true),
        )
        .layer(CompressionLayer::new())
        .with_state(state)
}

pub async fn serve(state: Arc<state::AppState>) -> anyhow::Result<()> {
    let host = state.cfg.server.host.clone();
    let port = state.cfg.server.port;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("parse bind address")?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(bind = %addr, "server listening");

    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown::shutdown_signal())
    .await?;
    Ok(())
}
