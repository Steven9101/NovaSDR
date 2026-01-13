use crate::state::AppState;
use axum::{
    extract::connect_info::ConnectInfo,
    extract::{ws, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;

pub async fn upgrade(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let Some(ip_guard) = state.try_acquire_ws_ip(addr.ip()) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many connections from this IP",
        )
            .into_response();
    };
    if state.event_clients.len() >= state.cfg.limits.events {
        return (StatusCode::TOO_MANY_REQUESTS, "too many events clients").into_response();
    }
    ws.on_upgrade(|socket| handle(socket, state, ip_guard))
}

async fn handle(socket: ws::WebSocket, state: Arc<AppState>, _ip_guard: crate::state::WsIpGuard) {
    let client_id = state.alloc_client_id();
    tracing::info!(client_id, "events ws connected");
    let (tx, mut rx) = crate::state::text_channel();
    state.event_clients.insert(client_id, tx);

    let mut initial = state.event_info(true);
    if state.cfg.server.otherusers > 0 {
        let mut snapshot = std::collections::HashMap::new();
        for rx in state.receivers.values() {
            let rx_id = rx.receiver.id.as_str();
            for entry in rx.audio_clients.iter() {
                let p = entry.params.load();
                snapshot.insert(format!("{rx_id}:{}", entry.unique_id), (p.l, p.m, p.r));
            }
        }
        initial.signal_changes = Some(snapshot);
    }
    let initial_json = match serde_json::to_string(&initial) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(client_id, error = ?e, "failed to serialize initial events payload");
            "{}".to_string()
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    if ws_sender
        .send(ws::Message::Text(initial_json))
        .await
        .is_err()
    {
        state.event_clients.remove(&client_id);
        return;
    }

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender
                .send(ws::Message::Text(msg.as_ref().to_string()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_receiver.next().await {
        if matches!(msg, ws::Message::Close(_)) {
            break;
        }
    }

    state.event_clients.remove(&client_id);
    tracing::info!(client_id, "events ws disconnected");
    send_task.abort();
}
