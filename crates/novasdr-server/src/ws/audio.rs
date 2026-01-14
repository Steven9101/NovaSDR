use crate::state::{AgcSpeed, AppState, AudioClient, AudioParams};
use axum::{
    extract::connect_info::ConnectInfo,
    extract::{ws, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use novasdr_core::{
    config::AudioCompression,
    dsp::{
        agc::Agc,
        dc_blocker::DcBlocker,
        demod::{
            add_complex, add_f32, am_envelope, float_to_i16_centered, negate_complex, negate_f32,
            polar_discriminator_fm, sam_demod, DemodulationMode,
        },
    },
    util::generate_unique_id,
};
use num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner};
use rustfft::{Fft as RustFft, FftPlanner};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

fn with_audio_unique_id(basic_info: String, unique_id: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&basic_info) else {
        return basic_info;
    };
    if let serde_json::Value::Object(map) = &mut v {
        map.insert("audio_unique_id".to_string(), json!(unique_id));
    } else {
        return basic_info;
    }
    match serde_json::to_string(&v) {
        Ok(s) => s,
        Err(_) => basic_info,
    }
}

#[derive(Clone, Copy, Debug)]
struct SquelchFeatures {
    scaled_relative_variance: f32,
    active_bins: u16,
    max_active_run: u16,
    len: usize,
}

fn squelch_features(bins: &[Complex32]) -> SquelchFeatures {
    let n = bins.len();
    if n < 2 {
        return SquelchFeatures {
            scaled_relative_variance: 0.0,
            active_bins: 0,
            max_active_run: 0,
            len: n,
        };
    }

    let mut sum_p = 0.0f64;
    let mut sum_p2 = 0.0f64;
    for c in bins {
        let p = c.norm_sqr() as f64;
        sum_p += p;
        sum_p2 += p * p;
    }

    let inv_n = 1.0f64 / (n as f64);
    let mean = sum_p * inv_n;
    if mean <= 0.0 {
        return SquelchFeatures {
            scaled_relative_variance: 0.0,
            active_bins: 0,
            max_active_run: 0,
            len: n,
        };
    }

    // var = E[p^2] - (E[p])^2
    let mut var = (sum_p2 * inv_n) - (mean * mean);
    if var < 0.0 {
        var = 0.0;
    }

    let rv = var / (mean * mean);
    let scaled_relative_variance = ((rv - 1.0) * (n as f64).sqrt()) as f32;

    let active_threshold = mean * 2.0;
    let mut active_bins: u16 = 0;
    let mut max_active_run: u16 = 0;
    let mut active_run: u16 = 0;
    for c in bins {
        let p = c.norm_sqr() as f64;
        if p.is_finite() && p >= active_threshold {
            active_bins = active_bins.saturating_add(1);
            active_run = active_run.saturating_add(1);
            if active_run > max_active_run {
                max_active_run = active_run;
            }
        } else {
            active_run = 0;
        }
    }

    SquelchFeatures {
        scaled_relative_variance,
        active_bins,
        max_active_run,
        len: n,
    }
}

const AUDIO_FRAME_MAGIC: [u8; 4] = *b"NSDA";
const AUDIO_FRAME_VERSION: u8 = 1;
const AUDIO_FRAME_HEADER_LEN: usize = 36;

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum AudioWireCodec {
    AdpcmIma = 1,
}

fn build_audio_frame(
    codec: AudioWireCodec,
    frame_num: u64,
    l: i32,
    m: f64,
    r: i32,
    pwr: f32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(AUDIO_FRAME_HEADER_LEN + payload.len());
    out.extend_from_slice(&AUDIO_FRAME_MAGIC);
    out.push(AUDIO_FRAME_VERSION);
    out.push(codec as u8);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&frame_num.to_le_bytes());
    out.extend_from_slice(&l.to_le_bytes());
    out.extend_from_slice(&m.to_le_bytes());
    out.extend_from_slice(&r.to_le_bytes());
    out.extend_from_slice(&pwr.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

mod ima_adpcm {
    const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

    const STEP_TABLE: [i32; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
        66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
        408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878,
        2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
        8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086,
        29794, 32767,
    ];

    pub fn encode_block_i16_mono(samples: &[i16]) -> Vec<u8> {
        if samples.is_empty() {
            return Vec::new();
        }

        let mut predictor = samples[0] as i32;
        let mut index = if samples.len() >= 2 {
            let diff = (samples[1] as i32 - samples[0] as i32).abs();
            let mut best = 0usize;
            for (i, &step) in STEP_TABLE.iter().enumerate() {
                if step >= diff {
                    best = i;
                    break;
                }
                best = i;
            }
            best as i32
        } else {
            0i32
        };

        let codes = samples.len().saturating_sub(1);
        let mut out = Vec::with_capacity(6 + codes.div_ceil(2));
        out.extend_from_slice(&(samples[0]).to_le_bytes());
        out.push(index as u8);
        out.push(0);
        out.extend_from_slice(&(samples.len() as u16).to_le_bytes());

        let mut pending: Option<u8> = None;

        for &sample in &samples[1..] {
            let step = STEP_TABLE[index as usize];
            let diff = (sample as i32) - predictor;
            let sign = if diff < 0 { 8 } else { 0 };
            let mut delta = diff.abs();

            let mut code = 0i32;
            let mut vpdiff = step >> 3;
            if delta >= step {
                code |= 4;
                delta -= step;
                vpdiff += step;
            }
            if delta >= (step >> 1) {
                code |= 2;
                delta -= step >> 1;
                vpdiff += step >> 1;
            }
            if delta >= (step >> 2) {
                code |= 1;
                vpdiff += step >> 2;
            }

            if sign != 0 {
                predictor -= vpdiff;
            } else {
                predictor += vpdiff;
            }
            predictor = predictor.clamp(i16::MIN as i32, i16::MAX as i32);

            code |= sign;
            index += INDEX_TABLE[code as usize];
            index = index.clamp(0, (STEP_TABLE.len() - 1) as i32);

            let nibble = (code as u8) & 0x0f;
            match pending.take() {
                Some(low) => out.push(low | (nibble << 4)),
                None => pending = Some(nibble),
            }
        }

        if let Some(low) = pending {
            out.push(low);
        }

        out
    }
}

#[derive(Debug, Clone)]
struct SquelchState {
    was_enabled: bool,
    open: bool,
    low_hits: u8,
    close_hits: u8,
}

impl SquelchState {
    fn new() -> Self {
        Self {
            was_enabled: false,
            open: true,
            low_hits: 0,
            close_hits: 0,
        }
    }

    fn reset_closed(&mut self) {
        self.open = false;
        self.low_hits = 0;
        self.close_hits = 0;
    }

    fn reset_open(&mut self) {
        self.open = true;
        self.low_hits = 0;
        self.close_hits = 0;
    }

    fn update(&mut self, enabled: bool, features: SquelchFeatures) -> bool {
        if enabled && !self.was_enabled {
            self.reset_closed();
        }
        if !enabled && self.was_enabled {
            self.reset_open();
        }
        self.was_enabled = enabled;
        if !enabled {
            return true;
        }

        let min_active_bins = if features.len <= 256 {
            1u16
        } else {
            ((features.len / 512).clamp(2, 6)) as u16
        };
        let active_enough = features.active_bins >= min_active_bins;

        let open_now = features.scaled_relative_variance >= 18.0 && active_enough;
        let open_soft = features.scaled_relative_variance >= 5.0 && active_enough;

        if open_now {
            self.open = true;
            self.low_hits = 0;
            self.close_hits = 0;
            return true;
        }

        if !self.open {
            if open_soft {
                self.low_hits = self.low_hits.saturating_add(1);
            } else {
                self.low_hits = 0;
            }
            if self.low_hits >= 3 {
                self.open = true;
                self.low_hits = 0;
                self.close_hits = 0;
            }
            return self.open;
        }

        // Close hysteresis: require sustained low variation before closing. Also close if the
        // slice is dominated by too few bins (narrow spurs/tones), or if activity is too sparse
        // (typical "static" with no concentrated signal energy).
        let min_active_run = if features.len <= 128 {
            1u16
        } else {
            ((features.len / 256).clamp(2, 8)) as u16
        };
        let run_enough = features.max_active_run >= min_active_run;

        if features.scaled_relative_variance < 2.0 || !active_enough || !run_enough {
            self.close_hits = self.close_hits.saturating_add(1);
        } else {
            self.close_hits = 0;
        }
        if self.close_hits >= 10 {
            self.reset_closed();
        }
        self.open
    }
}

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
    if state.total_audio_clients() >= state.cfg.limits.audio {
        return (StatusCode::TOO_MANY_REQUESTS, "too many audio clients").into_response();
    }
    ws.on_upgrade(|socket| handle(socket, state, ip_guard))
}

enum AudioOutbound {
    Switch { settings_json: String },
}

async fn handle(socket: ws::WebSocket, state: Arc<AppState>, _ip_guard: crate::state::WsIpGuard) {
    let client_id = state.alloc_client_id();
    tracing::info!(client_id, "audio ws connected");

    let mut receiver_id = state.active_receiver_id().to_string();
    let mut receiver = state.active_receiver_state().clone();

    let audio_fft_size = receiver.rt.audio_max_fft_size;
    let sample_rate = receiver.rt.audio_max_sps as usize;
    let compression = receiver.receiver.input.audio_compression;
    let pipeline = match AudioPipeline::new(sample_rate, audio_fft_size, compression) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                client_id,
                receiver_id = %receiver_id,
                sample_rate,
                audio_fft_size,
                error = ?e,
                "audio pipeline init failed"
            );
            return;
        }
    };

    let (tx, mut audio_rx) = crate::state::audio_channel();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<AudioOutbound>(8);

    let unique_id = generate_unique_id();
    let params = AudioParams {
        l: receiver.rt.default_l,
        m: receiver.rt.default_m,
        r: receiver.rt.default_r,
        mute: false,
        squelch_enabled: receiver.receiver.input.defaults.squelch_enabled,
        demodulation: DemodulationMode::from_str_upper(receiver.rt.default_mode_str.as_str())
            .unwrap_or(DemodulationMode::Usb),
        agc_speed: AgcSpeed::Default,
        agc_attack_ms: None,
        agc_release_ms: None,
    };
    let client = Arc::new(AudioClient {
        unique_id: unique_id.clone(),
        tx,
        params: std::sync::Mutex::new(params),
        pipeline: std::sync::Mutex::new(pipeline),
    });

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                Some(cmd) = out_rx.recv() => {
                    match cmd {
                        AudioOutbound::Switch { settings_json } => {
                            while audio_rx.try_recv().is_ok() {}
                            if ws_sender.send(ws::Message::Text(settings_json)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Some(bytes) = audio_rx.recv() => {
                    if ws_sender.send(ws::Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    let basic_info = with_audio_unique_id(
        state.basic_info_json(receiver_id.as_str()).await,
        &unique_id,
    );
    if out_tx
        .send(AudioOutbound::Switch {
            settings_json: basic_info,
        })
        .await
        .is_err()
    {
        send_task.abort();
        return;
    }

    receiver.audio_clients.insert(client_id, client.clone());
    state.broadcast_signal_changes(
        receiver_id.as_str(),
        &unique_id,
        receiver.rt.default_l,
        receiver.rt.default_m,
        receiver.rt.default_r,
    );

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

                        if next_id == receiver_id {
                            let settings_json = with_audio_unique_id(
                                state.basic_info_json(receiver_id.as_str()).await,
                                &unique_id,
                            );
                            if let Ok(mut p) = client.params.lock() {
                                p.l = receiver.rt.default_l;
                                p.m = receiver.rt.default_m;
                                p.r = receiver.rt.default_r;
                                p.mute = false;
                                p.squelch_enabled =
                                    receiver.receiver.input.defaults.squelch_enabled;
                                p.demodulation = DemodulationMode::from_str_upper(
                                    receiver.rt.default_mode_str.as_str(),
                                )
                                .unwrap_or(DemodulationMode::Usb);
                                p.agc_speed = AgcSpeed::Default;
                                p.agc_attack_ms = None;
                                p.agc_release_ms = None;
                            }
                            state.broadcast_signal_changes(
                                receiver_id.as_str(),
                                &unique_id,
                                receiver.rt.default_l,
                                receiver.rt.default_m,
                                receiver.rt.default_r,
                            );

                            if out_tx
                                .send(AudioOutbound::Switch { settings_json })
                                .await
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        let Some(next_receiver) = state.receiver_state(next_id.as_str()).cloned()
                        else {
                            continue;
                        };

                        let next_audio_fft_size = next_receiver.rt.audio_max_fft_size;
                        let next_sample_rate = next_receiver.rt.audio_max_sps as usize;
                        let next_compression = next_receiver.receiver.input.audio_compression;
                        let next_pipeline = match AudioPipeline::new(
                            next_sample_rate,
                            next_audio_fft_size,
                            next_compression,
                        ) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(receiver_id = %next_id, error = ?e, "failed to build audio pipeline for receiver switch");
                                continue;
                            }
                        };
                        let next_basic_info = with_audio_unique_id(
                            state.basic_info_json(next_id.as_str()).await,
                            &unique_id,
                        );

                        let old_receiver_id = receiver_id.clone();
                        receiver.audio_clients.remove(&client_id);
                        next_receiver
                            .audio_clients
                            .insert(client_id, client.clone());
                        receiver_id = next_id;
                        receiver = next_receiver;

                        {
                            let mut p = match client.params.lock() {
                                Ok(g) => g,
                                Err(poisoned) => {
                                    tracing::error!(
                                        unique_id = %client.unique_id,
                                        "audio params mutex poisoned; recovering"
                                    );
                                    poisoned.into_inner()
                                }
                            };
                            p.l = receiver.rt.default_l;
                            p.m = receiver.rt.default_m;
                            p.r = receiver.rt.default_r;
                            p.demodulation = DemodulationMode::from_str_upper(
                                receiver.rt.default_mode_str.as_str(),
                            )
                            .unwrap_or(DemodulationMode::Usb);
                        }
                        {
                            let mut pipeline = match client.pipeline.lock() {
                                Ok(g) => g,
                                Err(poisoned) => {
                                    tracing::error!(
                                        unique_id = %client.unique_id,
                                        "audio pipeline mutex poisoned; recovering"
                                    );
                                    poisoned.into_inner()
                                }
                            };
                            *pipeline = next_pipeline;
                        }

                        state.broadcast_signal_changes(
                            old_receiver_id.as_str(),
                            &unique_id,
                            -1,
                            -1.0,
                            -1,
                        );
                        state.broadcast_signal_changes(
                            receiver_id.as_str(),
                            &unique_id,
                            receiver.rt.default_l,
                            receiver.rt.default_m,
                            receiver.rt.default_r,
                        );

                        if out_tx
                            .send(AudioOutbound::Switch {
                                settings_json: next_basic_info,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    other => {
                        apply_command(&state, receiver_id.as_str(), &receiver, &client, other);
                    }
                }
            }
            ws::Message::Binary(_) => {}
            ws::Message::Close(_) => break,
            _ => {}
        }
    }

    receiver.audio_clients.remove(&client_id);
    state.broadcast_signal_changes(receiver_id.as_str(), &unique_id, -1, -1.0, -1);
    tracing::info!(client_id, %unique_id, "audio ws disconnected");
    send_task.abort();
}

fn apply_command(
    state: &Arc<AppState>,
    receiver_id: &str,
    receiver: &Arc<crate::state::ReceiverState>,
    client: &Arc<AudioClient>,
    cmd: novasdr_core::protocol::ClientCommand,
) {
    let rt = receiver.rt.as_ref();
    match cmd {
        novasdr_core::protocol::ClientCommand::Receiver { .. } => {}
        novasdr_core::protocol::ClientCommand::Window { l, r, m, .. } => {
            let Some(m) = m else { return };
            if l < 0 || r < 0 || l > r || r as usize >= rt.fft_result_size {
                return;
            }
            let audio_fft_size = rt.audio_max_fft_size as i32;
            if r - l > audio_fft_size {
                return;
            }
            let mut p = match client.params.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio params mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            p.l = l;
            p.r = r;
            p.m = m;
            state.broadcast_signal_changes(receiver_id, &client.unique_id, l, m, r);
        }
        novasdr_core::protocol::ClientCommand::Demodulation { demodulation } => {
            let mut p = match client.params.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio params mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            if let Some(mode) = DemodulationMode::from_str_upper(demodulation.as_str()) {
                p.demodulation = mode;
            }
            let mut pipeline = match client.pipeline.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio pipeline mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            pipeline.reset_agc();
        }
        novasdr_core::protocol::ClientCommand::Mute { mute } => {
            let mut p = match client.params.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio params mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            p.mute = mute;
        }
        novasdr_core::protocol::ClientCommand::Squelch { enabled } => {
            let mut p = match client.params.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio params mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            p.squelch_enabled = enabled;
        }
        novasdr_core::protocol::ClientCommand::Agc {
            speed,
            attack,
            release,
        } => {
            let mut p = match client.params.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!(
                        unique_id = %client.unique_id,
                        "audio params mutex poisoned; recovering"
                    );
                    poisoned.into_inner()
                }
            };
            p.agc_speed = AgcSpeed::parse(speed.as_str());
            p.agc_attack_ms = attack;
            p.agc_release_ms = release;
        }
        novasdr_core::protocol::ClientCommand::Userid { .. } => {}
        novasdr_core::protocol::ClientCommand::Buffer { .. } => {}
        novasdr_core::protocol::ClientCommand::Chat { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features_for_test(scaled_relative_variance: f32) -> SquelchFeatures {
        SquelchFeatures {
            scaled_relative_variance,
            active_bins: 64,
            max_active_run: 32,
            len: 1024,
        }
    }

    #[test]
    fn scaled_relative_variance_power_is_zero_for_empty_or_dc() {
        assert_eq!(squelch_features(&[]).scaled_relative_variance, 0.0);
        assert_eq!(
            squelch_features(&[Complex32::new(1.0, 0.0)]).scaled_relative_variance,
            0.0
        );
        let bins = vec![Complex32::new(2.0, 0.0); 128];
        let scaled = squelch_features(&bins).scaled_relative_variance;
        let expected = -((bins.len() as f32).sqrt());
        assert!(
            (scaled - expected).abs() < 1e-3,
            "expected scaled ~ {expected}, got {scaled}"
        );
    }

    #[test]
    fn squelch_disabled_is_always_open() {
        let mut s = SquelchState::new();
        for v in [0.0, 1.0, 10.0, 100.0] {
            assert!(s.update(false, features_for_test(v)));
        }
    }

    #[test]
    fn squelch_closes_after_sustained_low_variation() {
        let mut s = SquelchState::new();
        assert!(
            s.update(true, features_for_test(20.0)),
            "strong variation should open squelch"
        );
        for _ in 0..9 {
            assert!(
                s.update(true, features_for_test(0.0)),
                "should remain open until close hysteresis triggers"
            );
        }
        assert!(
            !s.update(true, features_for_test(0.0)),
            "should close after sustained low variance"
        );
    }

    #[test]
    fn squelch_opens_immediately_on_strong_variation() {
        let mut s = SquelchState::new();
        assert!(!s.update(true, features_for_test(0.0)));
        assert!(s.update(true, features_for_test(100.0)));
    }
}

pub struct AudioPipeline {
    compression: AudioCompression,
    audio_rate: usize,
    audio_fft_size: usize,
    ifft: Arc<dyn RustFft<f32>>,
    c2r_ifft: Arc<dyn ComplexToReal<f32>>,
    c2r_scratch: Vec<Complex32>,
    scratch: Vec<Complex32>,
    buf_in: Vec<Complex32>,
    baseband: Vec<Complex32>,
    carrier: Vec<Complex32>,
    baseband_prev: Vec<Complex32>,
    carrier_prev: Vec<Complex32>,
    real: Vec<f32>,
    real_prev: Vec<f32>,
    pcm_frame_i16: Vec<i16>,
    pcm_accum_i16: Vec<i16>,
    pcm_accum_offset: usize,
    packet_samples: usize,
    pwr_sum: f32,
    pwr_frames: usize,
    dc: DcBlocker,
    agc: Agc,
    fm_prev: Complex32,
    last_agc: (AgcSpeed, Option<f32>, Option<f32>),
    squelch: SquelchState,
}

impl AudioPipeline {
    pub fn new(
        sample_rate: usize,
        audio_fft_size: usize,
        compression: AudioCompression,
    ) -> anyhow::Result<Self> {
        let mut planner = FftPlanner::<f32>::new();
        let ifft = planner.plan_fft_inverse(audio_fft_size);

        let mut real_planner = RealFftPlanner::<f32>::new();
        let c2r_ifft = real_planner.plan_fft_inverse(audio_fft_size);
        let c2r_scratch = c2r_ifft.make_scratch_vec();

        let frame_samples = audio_fft_size / 2;

        // Batch ~20ms of PCM per websocket frame to reduce packet rate and browser-side scheduling
        // overhead (too many tiny frames can stutter).
        let target_packet_sec = 0.020_f64;
        let min_packet = ((sample_rate as f64) * target_packet_sec).ceil().max(1.0) as usize;
        let mut packet_samples = frame_samples.max(min_packet);
        packet_samples = packet_samples.div_ceil(8) * 8;
        packet_samples = packet_samples.clamp(frame_samples, 8192);

        Ok(Self {
            compression,
            audio_rate: sample_rate,
            audio_fft_size,
            ifft,
            c2r_ifft,
            c2r_scratch,
            scratch: vec![Complex32::new(0.0, 0.0); audio_fft_size],
            buf_in: vec![Complex32::new(0.0, 0.0); audio_fft_size],
            baseband: vec![Complex32::new(0.0, 0.0); audio_fft_size],
            carrier: vec![Complex32::new(0.0, 0.0); audio_fft_size],
            baseband_prev: vec![Complex32::new(0.0, 0.0); frame_samples],
            carrier_prev: vec![Complex32::new(0.0, 0.0); frame_samples],
            real: vec![0.0; audio_fft_size],
            real_prev: vec![0.0; frame_samples],
            pcm_frame_i16: vec![0; frame_samples],
            pcm_accum_i16: Vec::with_capacity(packet_samples * 4),
            pcm_accum_offset: 0,
            packet_samples,
            pwr_sum: 0.0,
            pwr_frames: 0,
            // Keep the DC blocker cutoff low so AM has real low end; bass boost is frontend-only.
            dc: DcBlocker::new((sample_rate / 20).max(128)),
            // Match reference defaults.
            agc: Agc::new(0.1, 100.0, 30.0, 100.0, sample_rate as f32),
            fm_prev: Complex32::new(0.0, 0.0),
            last_agc: (AgcSpeed::Default, None, None),
            squelch: SquelchState::new(),
        })
    }

    pub fn reset_agc(&mut self) {
        self.agc.reset();
    }

    fn reset_for_squelch_gate(&mut self) {
        self.real_prev.fill(0.0);
        self.baseband_prev.fill(Complex32::new(0.0, 0.0));
        self.carrier_prev.fill(Complex32::new(0.0, 0.0));
        self.fm_prev = Complex32::new(0.0, 0.0);
        self.dc.reset();
        self.agc.reset();
        self.pcm_accum_i16.clear();
        self.pcm_accum_offset = 0;
        self.pwr_sum = 0.0;
        self.pwr_frames = 0;
    }

    pub fn process(
        &mut self,
        spectrum_slice: &[Complex32],
        frame_num: u64,
        params: &AudioParams,
        is_real_input: bool,
        audio_mid_idx: i32,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut out_packets = Vec::new();
        if params.mute {
            return Ok(out_packets);
        }

        let features = squelch_features(spectrum_slice);
        let squelch_open = self.squelch.update(params.squelch_enabled, features);
        if params.squelch_enabled && !squelch_open {
            self.reset_for_squelch_gate();
            return Ok(out_packets);
        }

        let len = spectrum_slice.len() as i32;
        let audio_m_rel = (params.m.floor() as i32) - params.l;

        let mode = params.demodulation;

        let n = self.audio_fft_size as i32;
        let half = (self.audio_fft_size / 2) as i32;

        match mode {
            DemodulationMode::Usb | DemodulationMode::Lsb => {
                // C2R IFFT input: N/2+1 complex values in hermitian format
                let c2r_len = self.audio_fft_size / 2 + 1;
                self.buf_in[..c2r_len].fill(Complex32::new(0.0, 0.0));

                if mode == DemodulationMode::Usb {
                    let copy_l = 0.max(audio_m_rel);
                    let copy_r = len.min(audio_m_rel + n);
                    if copy_r >= copy_l {
                        for i in copy_l..copy_r {
                            let dst = (i - audio_m_rel) as usize;
                            if dst < c2r_len {
                                self.buf_in[dst] = spectrum_slice[i as usize];
                            }
                        }
                    }
                } else {
                    let copy_l = 0.max(audio_m_rel - n + 1);
                    let copy_r = len.min(audio_m_rel + 1);
                    if copy_r >= copy_l {
                        let dst0 = (audio_m_rel - copy_r + 1) as usize;
                        let count = (copy_r - copy_l) as usize;
                        for k in 0..count {
                            let dst = dst0 + k;
                            if dst < c2r_len {
                                self.buf_in[dst] = spectrum_slice[(copy_r as usize) - 1 - k];
                            }
                        }
                    }
                }

                let _ = self.c2r_ifft.process_with_scratch(
                    &mut self.buf_in[..c2r_len],
                    &mut self.real,
                    &mut self.c2r_scratch,
                );

                if mode == DemodulationMode::Lsb {
                    self.real.reverse();
                }

                if frame_num % 2 == 1
                    && (((audio_mid_idx % 2 == 0) && !is_real_input)
                        || ((audio_mid_idx % 2 != 0) && is_real_input))
                {
                    negate_f32(&mut self.real);
                }
                add_f32(&mut self.real[..self.audio_fft_size / 2], &self.real_prev);
            }
            DemodulationMode::Am | DemodulationMode::Sam | DemodulationMode::Fm => {
                let need_carrier = mode == DemodulationMode::Sam;

                self.buf_in.fill(Complex32::new(0.0, 0.0));
                let pos_copy_l = 0.max(audio_m_rel);
                let pos_copy_r = len.min(audio_m_rel + half);
                if pos_copy_r >= pos_copy_l {
                    for i in pos_copy_l..pos_copy_r {
                        let dst = (i - audio_m_rel) as usize;
                        self.buf_in[dst] = spectrum_slice[i as usize];
                    }
                }
                let neg_copy_l = 0.max(audio_m_rel - half + 1);
                let neg_copy_r = len.min(audio_m_rel);
                if neg_copy_r >= neg_copy_l {
                    for i in neg_copy_l..neg_copy_r {
                        let dst = (self.audio_fft_size as i32 - (audio_m_rel - i)) as usize;
                        if dst < self.buf_in.len() {
                            self.buf_in[dst] = spectrum_slice[i as usize];
                        }
                    }
                }

                self.baseband.copy_from_slice(&self.buf_in);
                self.ifft
                    .process_with_scratch(&mut self.baseband, &mut self.scratch);

                if need_carrier {
                    self.carrier.copy_from_slice(&self.buf_in);
                    let cutoff =
                        (500 * self.audio_fft_size / self.audio_rate).min(self.audio_fft_size / 2);
                    for i in cutoff..(self.audio_fft_size - cutoff) {
                        self.carrier[i] = Complex32::new(0.0, 0.0);
                    }
                    self.ifft
                        .process_with_scratch(&mut self.carrier, &mut self.scratch);
                }

                if frame_num % 2 == 1
                    && (((audio_mid_idx % 2 == 0) && !is_real_input)
                        || ((audio_mid_idx % 2 != 0) && is_real_input))
                {
                    negate_complex(&mut self.baseband);
                    if need_carrier {
                        negate_complex(&mut self.carrier);
                    }
                }

                add_complex(
                    &mut self.baseband[..self.audio_fft_size / 2],
                    &self.baseband_prev,
                );
                if need_carrier {
                    add_complex(
                        &mut self.carrier[..self.audio_fft_size / 2],
                        &self.carrier_prev,
                    );
                }

                match mode {
                    DemodulationMode::Am => {
                        am_envelope(
                            &self.baseband[..self.audio_fft_size / 2],
                            &mut self.real[..self.audio_fft_size / 2],
                        );
                    }
                    DemodulationMode::Sam => {
                        sam_demod(
                            &self.baseband[..self.audio_fft_size / 2],
                            &self.carrier[..self.audio_fft_size / 2],
                            &mut self.real[..self.audio_fft_size / 2],
                        );
                    }
                    DemodulationMode::Fm => {
                        self.fm_prev = polar_discriminator_fm(
                            &self.baseband[..self.audio_fft_size / 2],
                            self.fm_prev,
                            &mut self.real[..self.audio_fft_size / 2],
                        );
                    }
                    _ => {}
                }
                self.real[self.audio_fft_size / 2..].fill(0.0);
            }
        }

        self.real_prev
            .copy_from_slice(&self.real[self.audio_fft_size / 2..]);
        self.baseband_prev
            .copy_from_slice(&self.baseband[self.audio_fft_size / 2..]);
        if mode == DemodulationMode::Sam {
            self.carrier_prev
                .copy_from_slice(&self.carrier[self.audio_fft_size / 2..]);
        }

        self.apply_agc_settings(params);

        let half = self.audio_fft_size / 2;
        let audio_out = &mut self.real[..half];
        self.dc.remove_dc(audio_out);
        self.agc.process(audio_out);

        float_to_i16_centered(audio_out, &mut self.pcm_frame_i16, 32768.0);
        self.pcm_accum_i16.extend_from_slice(&self.pcm_frame_i16);
        self.pwr_sum += spectrum_slice.iter().map(|c| c.norm_sqr()).sum::<f32>();
        self.pwr_frames += 1;

        if self.compression != AudioCompression::Adpcm {
            return Err(anyhow::anyhow!(
                "FLAC audio was removed; configure audio_compression = \"adpcm\""
            ));
        }

        loop {
            let available = self
                .pcm_accum_i16
                .len()
                .saturating_sub(self.pcm_accum_offset);
            if available < self.packet_samples {
                break;
            }

            let end = self.pcm_accum_offset + self.packet_samples;
            let block = &self.pcm_accum_i16[self.pcm_accum_offset..end];
            self.pcm_accum_offset = end;

            let frames = self.pwr_frames.max(1) as f32;
            let pwr = self.pwr_sum / frames;
            self.pwr_sum = 0.0;
            self.pwr_frames = 0;

            let payload = ima_adpcm::encode_block_i16_mono(block);
            out_packets.push(build_audio_frame(
                AudioWireCodec::AdpcmIma,
                frame_num,
                0,
                params.m,
                spectrum_slice.len() as i32,
                pwr,
                &payload,
            ));

            if self.pcm_accum_offset >= self.packet_samples * 4 {
                self.pcm_accum_i16.drain(0..self.pcm_accum_offset);
                self.pcm_accum_offset = 0;
            }
        }
        Ok(out_packets)
    }

    fn apply_agc_settings(&mut self, params: &AudioParams) {
        let current = (
            params.agc_speed,
            params.agc_attack_ms,
            params.agc_release_ms,
        );
        if current == self.last_agc {
            return;
        }
        self.last_agc = current;

        let (speed, attack_ms, release_ms) = current;
        let (attack_s, release_s) = match speed {
            AgcSpeed::Custom => match (attack_ms, release_ms) {
                (Some(a), Some(r)) => ((a / 1000.0).max(0.0001), (r / 1000.0).max(0.0001)),
                _ => (0.003, 0.25),
            },
            AgcSpeed::Off => (0.0001, 0.0001),
            AgcSpeed::Fast => (0.001, 0.05),
            AgcSpeed::Slow => (0.05, 0.5),
            AgcSpeed::Medium => (0.01, 0.15),
            AgcSpeed::Default => (0.003, 0.25),
        };

        let sr = self.audio_rate as f32;
        let attack_coeff = 1.0 - (-1.0 / (attack_s * sr)).exp();
        let release_coeff = 1.0 - (-1.0 / (release_s * sr)).exp();
        self.agc.set_attack_coeff(attack_coeff);
        self.agc.set_release_coeff(release_coeff);
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use realfft::RealFftPlanner;

    #[test]
    fn realfft_inverse_is_unnormalized_like_fftw_backward() {
        // FFTW's BACKWARD inverse does not normalize by 1/N.
        // Our audio pipeline relies on matching that scaling.
        let n = 8usize;
        let mut planner = RealFftPlanner::<f32>::new();
        let ifft = planner.plan_fft_inverse(n);
        let mut scratch = ifft.make_scratch_vec();

        // Hermitian format length: N/2 + 1
        let mut spectrum = vec![Complex32::new(0.0, 0.0); n / 2 + 1];
        spectrum[0] = Complex32::new(1.0, 0.0); // DC = 1.0

        let mut time = vec![0.0f32; n];
        let _ = ifft.process_with_scratch(&mut spectrum, &mut time, &mut scratch);

        // Unnormalized inverse: DC=1.0 -> constant 1.0 in time domain.
        // Normalized inverse would produce 1.0 / N.
        for v in time {
            assert!(
                (v - 1.0).abs() < 1e-4,
                "expected unnormalized inverse (1.0), got {v}"
            );
        }
    }

    #[test]
    fn scaled_relative_variance_is_near_zero_for_rv_one() {
        // Construct powers [0, 2] -> mean=1, var=1 -> rv=1 -> scaled=0.
        let bins = [
            Complex32::new(0.0, 0.0),
            Complex32::new(2.0_f32.sqrt(), 0.0),
        ];
        let scaled = squelch_features(&bins).scaled_relative_variance;
        assert!(scaled.abs() < 1e-4, "expected scaled near 0, got {scaled}");
    }

    #[test]
    fn scaled_relative_variance_is_large_for_single_bin_spike() {
        // For N bins, powers [1, 0, 0, ...] yields rv = N-1 and scaled = (N-2)*sqrt(N).
        let mut bins = vec![Complex32::new(0.0, 0.0); 64];
        bins[0] = Complex32::new(1.0, 0.0);
        let scaled = squelch_features(&bins).scaled_relative_variance;
        assert!(
            scaled > 100.0,
            "expected scaled to be large for a single-bin spike, got {scaled}"
        );
    }

    #[test]
    fn squelch_state_machine_opens_on_consecutive_soft_hits_and_closes_with_hysteresis() {
        let mut s = SquelchState::new();
        let features = |scaled_relative_variance: f32| -> SquelchFeatures {
            SquelchFeatures {
                scaled_relative_variance,
                active_bins: 64,
                max_active_run: 32,
                len: 1024,
            }
        };

        // Enabling squelch closes it until a signal is detected.
        assert!(
            !s.update(true, features(0.0)),
            "expected closed immediately after enable"
        );

        // Soft open: scaled >= 5 for 3 consecutive frames.
        assert!(!s.update(true, features(6.0)));
        assert!(!s.update(true, features(6.0)));
        assert!(
            s.update(true, features(6.0)),
            "expected open after 3 consecutive soft hits"
        );

        // Close hysteresis: scaled < 2 for 10 consecutive frames.
        for _ in 0..9 {
            assert!(
                s.update(true, features(1.0)),
                "expected to remain open during close hysteresis"
            );
        }
        assert!(
            !s.update(true, features(1.0)),
            "expected to close after hysteresis completes"
        );
    }
}
