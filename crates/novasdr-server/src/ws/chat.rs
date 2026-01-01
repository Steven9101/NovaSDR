use crate::state::{append_chat_message, AppState, ChatMessage};
use axum::{
    extract::connect_info::ConnectInfo,
    extract::{ws, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use novasdr_core::protocol::ClientCommand;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::Instant;

pub async fn upgrade(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    if !state.cfg.websdr.chat_enabled {
        return (StatusCode::NOT_FOUND, "chat disabled").into_response();
    }
    let Some(ip_guard) = state.try_acquire_ws_ip(addr.ip()) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many connections from this IP",
        )
            .into_response();
    };
    ws.on_upgrade(|socket| handle(socket, state, ip_guard))
}

async fn handle(socket: ws::WebSocket, state: Arc<AppState>, _ip_guard: crate::state::WsIpGuard) {
    let client_id = state.alloc_client_id();
    tracing::info!(client_id, "chat ws connected");
    let (tx, mut rx) = crate::state::text_channel();
    state.chat_clients.insert(client_id, tx);

    let history = {
        let hist = state.chat_history.lock().await;
        hist.clone()
    };
    let history_msg = match serde_json::to_string(&serde_json::json!({
        "type": "history",
        "messages": history
    })) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(client_id, error = ?e, "failed to serialize chat history");
            "{\"type\":\"history\",\"messages\":[]}".to_string()
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    if ws_sender
        .send(ws::Message::Text(history_msg))
        .await
        .is_err()
    {
        state.chat_clients.remove(&client_id);
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

    let mut window_start = Instant::now();
    let mut msgs_in_window: u32 = 0;
    let mut rate_violations: u32 = 0;

    while let Some(Ok(msg)) = ws_receiver.next().await {
        let ws::Message::Text(txt) = msg else {
            continue;
        };
        // Simple spam guard: drop bursts; disconnect on repeated violations.
        let now = Instant::now();
        if now.duration_since(window_start).as_secs_f32() >= 10.0 {
            window_start = now;
            msgs_in_window = 0;
        }
        msgs_in_window = msgs_in_window.saturating_add(1);
        if msgs_in_window > 20 {
            rate_violations = rate_violations.saturating_add(1);
            if rate_violations == 1 || rate_violations.is_power_of_two() {
                tracing::warn!(
                    client_id,
                    msgs_in_window,
                    rate_violations,
                    "chat rate limit exceeded; dropping messages"
                );
            }
            if rate_violations >= 8 {
                break;
            }
            continue;
        }
        if txt.len() > 1024 {
            continue;
        }
        let Ok(cmd) = serde_json::from_str::<ClientCommand>(&txt) else {
            continue;
        };
        if let ClientCommand::Chat {
            message,
            username,
            user_id,
            reply_to_id,
            reply_to_username,
        } = cmd
        {
            let user_id = user_id.unwrap_or_else(|| format!("legacy_{client_id}"));
            if let Some(chat_msg) = build_chat_message(
                &user_id,
                &username,
                &message,
                reply_to_id.unwrap_or_default(),
                reply_to_username.unwrap_or_default(),
            ) {
                let json_msg = match serde_json::to_string(&chat_msg) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(client_id, error = ?e, "failed to serialize chat message");
                        continue;
                    }
                };
                append_chat_message(&state, chat_msg.clone()).await;
                let msg: Arc<str> = Arc::from(json_msg);
                let mut dead = Vec::new();
                for entry in state.chat_clients.iter() {
                    if entry.value().try_send(msg.clone()).is_err() {
                        dead.push(*entry.key());
                    }
                }
                for id in dead {
                    state.chat_clients.remove(&id);
                }
            }
        }
    }

    state.chat_clients.remove(&client_id);
    tracing::info!(client_id, "chat ws disconnected");
    send_task.abort();
}

fn build_chat_message(
    user_id: &str,
    username: &str,
    message: &str,
    reply_to_id: String,
    reply_to_username: String,
) -> Option<ChatMessage> {
    let mut username = username.trim().to_string();
    if username.is_empty() {
        username = "user".to_string();
    }
    if username.len() > 14 {
        username.truncate(14);
    }
    if is_blocked_username(&username) {
        username = "user".to_string();
    }

    let mut message = message.trim().to_string();
    if message.is_empty() {
        return None;
    }
    if message.len() > 200 {
        message.truncate(200);
    }
    message = filter_message(&message);

    let id = format!(
        "{}_{}",
        chrono::Utc::now().timestamp_millis(),
        rand::random::<u32>()
    );
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    Some(ChatMessage {
        id,
        username,
        message,
        timestamp,
        user_id: user_id.to_string(),
        r#type: "message".to_string(),
        reply_to_id,
        reply_to_username,
    })
}

fn is_blocked_username(username: &str) -> bool {
    static BLOCKED: &[&str] = &["admin", "operator", "host", "root", "system", "moderator"];
    BLOCKED.iter().any(|w| w.eq_ignore_ascii_case(username))
}

fn filter_message(message: &str) -> String {
    #[derive(Debug)]
    struct Filter {
        re: regex::Regex,
        replacement: String,
    }

    static FILTERS: std::sync::OnceLock<Vec<Filter>> = std::sync::OnceLock::new();
    let filters = FILTERS.get_or_init(|| {
        const WORDS: &[&str] = &[
            "fuck", "fucking", "bitch", "shit", "asshole", "cunt", "bastard", "idiot", "moron",
            "dumb", "stupid", "loser", "retard",
        ];
        let mut out = Vec::with_capacity(WORDS.len());
        for word in WORDS {
            let pat = format!(r"(?i)\b{}\b", regex::escape(word));
            match regex::Regex::new(&pat) {
                Ok(re) => out.push(Filter {
                    re,
                    replacement: "*".repeat(word.len()),
                }),
                Err(e) => {
                    tracing::error!(error = ?e, pattern = %pat, "failed to compile chat filter")
                }
            }
        }
        out
    });

    let mut out = message.to_string();
    for f in filters {
        out = f.re.replace_all(&out, f.replacement.as_str()).to_string();
    }
    out
}
