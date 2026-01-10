use crate::{shutdown, state, ws};
use anyhow::Context;
use axum::{routing::get, Router};
use std::{net::SocketAddr, sync::Arc, time::Duration, path::PathBuf};
use tower_http::{compression::CompressionLayer, services::ServeDir};

// neu
use axum_server::tls_rustls::RustlsConfig;


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

// Neu Port 80 und 443
pub async fn serve(state: Arc<state::AppState>) -> anyhow::Result<()> {
    // Host aus config
    let host_cfg = state.cfg.server.host.clone();
    let host = if host_cfg.contains(':') && !host_cfg.starts_with('[') {
        format!("[{host_cfg}]")
    } else {
        host_cfg
    };

    // Fix: 80 & 443
    let http_addr: SocketAddr = format!("{host}:80").parse().context("parse http bind")?;
    let https_addr: SocketAddr = format!("{host}:443").parse().context("parse https bind")?;

    // TLS muss gesetzt sein
    let tls = state
        .cfg
        .server
        .tls
        .as_ref()
        .context("server.tls fehlt (cert/key erforderlich für :443)")?;

    let rustls: RustlsConfig = RustlsConfig::from_pem_file(
        PathBuf::from(&tls.cert_pem_path),
        PathBuf::from(&tls.key_pem_path),
    )
    .await
    .context("load TLS PEM files")?;

    // Für beide Server je ein make_service (einfach und zuverlässig)
    let http_svc = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    let https_svc = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();

    // Graceful shutdown über axum-server Handle
    let http_handle = axum_server::Handle::new();
    let https_handle = axum_server::Handle::new();
    let http_handle_shutdown = http_handle.clone();
    let https_handle_shutdown = https_handle.clone();

    tokio::spawn(async move {
        shutdown::shutdown_signal().await;
        let grace = Some(Duration::from_secs(10));
        http_handle_shutdown.graceful_shutdown(grace);
        https_handle_shutdown.graceful_shutdown(grace);
    });

    tracing::info!(bind = %http_addr, "http server listening");
    tracing::info!(bind = %https_addr, "https server listening");

    // Beide Server parallel starten
    let http_task = tokio::spawn(async move {
        axum_server::bind(http_addr)
            .handle(http_handle)
            .serve(http_svc)
            .await
    });

    let https_task = tokio::spawn(async move {
        axum_server::bind_rustls(https_addr, rustls)
            .handle(https_handle)
            .serve(https_svc)
            .await
    });

    let (r1, r2) = tokio::join!(http_task, https_task);
    r1??;
    r2??;

    Ok(())
}
