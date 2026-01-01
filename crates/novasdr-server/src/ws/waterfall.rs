use crate::state::{AppState, ClientId, WaterfallClient, WaterfallParams};
use axum::{
    extract::connect_info::ConnectInfo,
    extract::{ws, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use novasdr_core::{codec::zstd_stream::ZstdStreamEncoder, protocol::WaterfallPacket};
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
    if state.total_waterfall_clients() >= state.cfg.limits.waterfall {
        return (StatusCode::TOO_MANY_REQUESTS, "too many waterfall clients").into_response();
    }
    ws.on_upgrade(|socket| handle(socket, state, ip_guard))
}

enum WaterfallOutbound {
    Switch { settings_json: String },
}

async fn handle(socket: ws::WebSocket, state: Arc<AppState>, _ip_guard: crate::state::WsIpGuard) {
    let client_id = state.alloc_client_id();
    tracing::info!(client_id, "waterfall ws connected");

    let mut receiver_id = state.active_receiver_id().to_string();
    let mut receiver = state.active_receiver_state().clone();

    let (tx, mut rx) = crate::state::waterfall_channel();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<WaterfallOutbound>(8);
    let encoder = match WaterfallEncoder::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(client_id, error = ?e, "waterfall encoder init failed");
            return;
        }
    };

    let initial_level = receiver.rt.downsample_levels - 1;
    let initial_l = 0usize;
    let initial_r = receiver.rt.min_waterfall_fft;

    let client = Arc::new(WaterfallClient {
        tx,
        params: std::sync::Mutex::new(WaterfallParams {
            level: initial_level,
            l: initial_l,
            r: initial_r,
        }),
    });

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let state_for_send = state.clone();
    let send_task = tokio::spawn(async move {
        let mut encoder = encoder;
        loop {
            tokio::select! {
                biased;
                Some(cmd) = out_rx.recv() => {
                    match cmd {
                        WaterfallOutbound::Switch { settings_json } => {
                            while rx.try_recv().is_ok() {}
                            encoder = match WaterfallEncoder::new() {
                                Ok(e) => e,
                                Err(e) => {
                                    tracing::error!(client_id, error = ?e, "waterfall encoder reinit failed");
                                    break;
                                }
                            };
                            if ws_sender.send(ws::Message::Text(settings_json)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Some(item) = rx.recv() => {
                    let want_len = item.r.saturating_sub(item.l);
                    let Some(end) = item.quantized_offset.checked_add(want_len) else {
                        tracing::warn!(
                            client_id,
                            offset = item.quantized_offset,
                            len = want_len,
                            "waterfall frame has invalid offset/len (overflow); dropping"
                        );
                        continue;
                    };
                    let Some(data) = item.quantized_concat.get(item.quantized_offset..end) else {
                        tracing::warn!(
                            client_id,
                            level = item.level,
                            l = item.l,
                            r = item.r,
                            offset = item.quantized_offset,
                            want_end = end,
                            buf_len = item.quantized_concat.len(),
                            "waterfall frame out of bounds; dropping"
                        );
                        continue;
                    };
                    let pkt = match encoder.encode(item.frame_num, item.level, item.l, item.r, data) {
                        Ok(pkt) => pkt,
                        Err(e) => {
                            tracing::warn!(client_id, error = ?e, "waterfall encode failed; dropping frame");
                            continue;
                        }
                    };

                    state_for_send
                        .total_waterfall_bits
                        .fetch_add(pkt.len() * 8, std::sync::atomic::Ordering::Relaxed);

                    if ws_sender.send(ws::Message::Binary(pkt)).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    let basic_info = state.basic_info_json(receiver_id.as_str()).await;
    if out_tx
        .send(WaterfallOutbound::Switch {
            settings_json: basic_info,
        })
        .await
        .is_err()
    {
        send_task.abort();
        return;
    }

    receiver.waterfall_clients[initial_level].insert(client_id, client.clone());

    while let Some(Ok(msg)) = ws_receiver.next().await {
        match msg {
            ws::Message::Text(txt) => {
                if txt.len() > 1024 {
                    continue;
                }
                let Ok(cmd) = serde_json::from_str::<novasdr_core::protocol::ClientCommand>(&txt)
                else {
                    continue;
                };
                match cmd {
                    novasdr_core::protocol::ClientCommand::Receiver {
                        receiver_id: next_id,
                    } => {
                        let next_id = next_id.trim().to_string();
                        if next_id.is_empty() {
                            continue;
                        }
                        let is_switch = next_id != receiver_id;
                        let Some(next_receiver) = state.receiver_state(next_id.as_str()).cloned()
                        else {
                            continue;
                        };
                        let next_basic_info = state.basic_info_json(next_id.as_str()).await;

                        let old_level = match client.params.lock() {
                            Ok(g) => g.level,
                            Err(poisoned) => {
                                tracing::error!(
                                    client_id,
                                    "waterfall params mutex poisoned; recovering"
                                );
                                poisoned.into_inner().level
                            }
                        };
                        receiver.waterfall_clients[old_level].remove(&client_id);

                        let next_initial_level = next_receiver.rt.downsample_levels - 1;
                        let next_initial_r = next_receiver.rt.min_waterfall_fft;
                        {
                            let mut p = match client.params.lock() {
                                Ok(g) => g,
                                Err(poisoned) => {
                                    tracing::error!(
                                        client_id,
                                        "waterfall params mutex poisoned; recovering"
                                    );
                                    poisoned.into_inner()
                                }
                            };
                            p.level = next_initial_level;
                            p.l = 0;
                            p.r = next_initial_r;
                        }

                        if out_tx
                            .send(WaterfallOutbound::Switch {
                                settings_json: next_basic_info,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }

                        if is_switch {
                            next_receiver.waterfall_clients[next_initial_level]
                                .insert(client_id, client.clone());
                            receiver_id = next_id;
                            receiver = next_receiver;
                        } else {
                            receiver.waterfall_clients[next_initial_level]
                                .insert(client_id, client.clone());
                        }
                    }
                    other => {
                        apply_command(&state, &receiver, client_id, &client, other);
                    }
                }
            }
            ws::Message::Close(_) => break,
            _ => {}
        }
    }

    let level = match client.params.lock() {
        Ok(g) => g.level,
        Err(poisoned) => {
            tracing::error!(client_id, "waterfall params mutex poisoned; recovering");
            poisoned.into_inner().level
        }
    };
    receiver.waterfall_clients[level].remove(&client_id);
    tracing::info!(client_id, "waterfall ws disconnected");
    send_task.abort();
}

fn apply_command(
    _state: &Arc<AppState>,
    receiver: &Arc<crate::state::ReceiverState>,
    client_id: ClientId,
    client: &Arc<WaterfallClient>,
    cmd: novasdr_core::protocol::ClientCommand,
) {
    let rt = receiver.rt.as_ref();
    let novasdr_core::protocol::ClientCommand::Window { l, r, .. } = cmd else {
        return;
    };

    if l < 0 || r < 0 || l >= r {
        return;
    }

    let mut new_l = l;
    let mut new_r = r;

    let downsample_levels = rt.downsample_levels as i32;
    let mut new_level = downsample_levels - 1;
    let mut best_diff = (rt.min_waterfall_fft as i32) * 2;
    let mut lf = new_l as f32;
    let mut rf = new_r as f32;
    for i in 0..downsample_levels {
        let send_size = ((rf - lf) - (rt.min_waterfall_fft as f32)).abs();
        if send_size < (best_diff as f32) {
            best_diff = send_size as i32;
            new_level = i;
            new_l = lf.round() as i32;
            new_r = rf.round() as i32;
        }
        lf /= 2.0;
        rf /= 2.0;
    }

    if new_l < 0 || new_r <= new_l {
        return;
    }
    let new_level_usize = new_level as usize;
    let new_l_usize = new_l as usize;
    let new_r_usize = new_r as usize;
    if new_r_usize > (rt.fft_result_size >> new_level_usize) {
        return;
    }

    let mut p = match client.params.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::error!(client_id, "waterfall params mutex poisoned; recovering");
            poisoned.into_inner()
        }
    };
    if p.level != new_level_usize {
        receiver.waterfall_clients[p.level].remove(&client_id);
        receiver.waterfall_clients[new_level_usize].insert(client_id, client.clone());
    }
    p.level = new_level_usize;
    p.l = new_l_usize;
    p.r = new_r_usize;
}

pub struct WaterfallEncoder {
    zstd: ZstdStreamEncoder,
}

impl WaterfallEncoder {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            zstd: ZstdStreamEncoder::new(3)?,
        })
    }

    pub fn encode(
        &mut self,
        frame_num: u64,
        level: usize,
        l: usize,
        r: usize,
        data: &[i8],
    ) -> anyhow::Result<Vec<u8>> {
        let pkt = WaterfallPacket {
            frame_num,
            l: (l << level) as i32,
            r: (r << level) as i32,
            data: bytemuck::cast_slice::<i8, u8>(data),
        };
        let cbor = serde_cbor::to_vec(&pkt)?;
        self.zstd.compress_flush(&cbor)
    }
}
