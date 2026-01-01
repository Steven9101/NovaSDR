use crate::state::{AgcSpeed, AppState, AudioClient, AudioParams};
use axum::{
    extract::connect_info::ConnectInfo,
    extract::{ws, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use novasdr_core::{
    codec::flac_stream::FlacStreamEncoder,
    dsp::{
        agc::Agc,
        dc_blocker::DcBlocker,
        demod::{
            add_complex, add_f32, am_envelope, float_to_i16_centered, negate_complex, negate_f32,
            polar_discriminator_fm, sam_demod, DemodulationMode,
        },
    },
    protocol::AudioPacket,
    util::generate_unique_id,
};
use num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner};
use rustfft::{Fft as RustFft, FftPlanner};
use std::net::SocketAddr;
use std::sync::Arc;

fn scaled_relative_variance_power(bins: &[Complex32]) -> f32 {
    let n = bins.len();
    if n < 2 {
        return 0.0;
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
        return 0.0;
    }

    // var = E[p^2] - (E[p])^2
    let mut var = (sum_p2 * inv_n) - (mean * mean);
    if var < 0.0 {
        var = 0.0;
    }

    let rv = var / (mean * mean);
    ((rv - 1.0) * (n as f64).sqrt()) as f32
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

    fn update(&mut self, enabled: bool, scaled_relative_variance: f32) -> bool {
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

        let open_now = scaled_relative_variance >= 18.0;
        let open_soft = scaled_relative_variance >= 5.0;

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

        // Close hysteresis: require sustained low variation before closing.
        if scaled_relative_variance < 2.0 {
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
    Switch {
        settings_json: String,
        header_pkt: Vec<u8>,
    },
}

async fn handle(socket: ws::WebSocket, state: Arc<AppState>, _ip_guard: crate::state::WsIpGuard) {
    let client_id = state.alloc_client_id();
    tracing::info!(client_id, "audio ws connected");

    let mut receiver_id = state.active_receiver_id().to_string();
    let mut receiver = state.active_receiver_state().clone();

    let audio_fft_size = receiver.rt.audio_max_fft_size;
    let sample_rate = receiver.rt.audio_max_sps as usize;
    let pipeline = match AudioPipeline::new(sample_rate, audio_fft_size) {
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
    let header_pkt = match pipeline.flac.header_bytes().ok().and_then(|header| {
        let pkt = AudioPacket {
            frame_num: 0,
            l: 0,
            m: 0.0,
            r: 0,
            pwr: 0.0,
            data: &header,
        };
        serde_cbor::to_vec(&pkt).ok()
    }) {
        Some(v) => v,
        None => {
            tracing::warn!(
                client_id,
                receiver_id = %receiver_id,
                "failed to build audio FLAC header packet"
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
        squelch_enabled: false,
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
                        AudioOutbound::Switch { settings_json, header_pkt } => {
                            while audio_rx.try_recv().is_ok() {}
                            if ws_sender.send(ws::Message::Text(settings_json)).await.is_err() {
                                break;
                            }
                            if ws_sender.send(ws::Message::Binary(header_pkt)).await.is_err() {
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

    let basic_info = state.basic_info_json(receiver_id.as_str()).await;
    if out_tx
        .send(AudioOutbound::Switch {
            settings_json: basic_info,
            header_pkt,
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
                            let settings_json = state.basic_info_json(receiver_id.as_str()).await;
                            let header_pkt = match client
                                .pipeline
                                .lock()
                                .ok()
                                .and_then(|p| p.flac.header_bytes().ok())
                                .and_then(|header| {
                                    let pkt = AudioPacket {
                                        frame_num: 0,
                                        l: 0,
                                        m: 0.0,
                                        r: 0,
                                        pwr: 0.0,
                                        data: &header,
                                    };
                                    serde_cbor::to_vec(&pkt).ok()
                                }) {
                                Some(v) => v,
                                None => continue,
                            };

                            if let Ok(mut p) = client.params.lock() {
                                p.l = receiver.rt.default_l;
                                p.m = receiver.rt.default_m;
                                p.r = receiver.rt.default_r;
                                p.mute = false;
                                p.squelch_enabled = false;
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
                                .send(AudioOutbound::Switch {
                                    settings_json,
                                    header_pkt,
                                })
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
                        let next_pipeline = match AudioPipeline::new(
                            next_sample_rate,
                            next_audio_fft_size,
                        ) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(receiver_id = %next_id, error = ?e, "failed to build audio pipeline for receiver switch");
                                continue;
                            }
                        };
                        let next_header_pkt =
                            match next_pipeline.flac.header_bytes().ok().and_then(|header| {
                                let pkt = AudioPacket {
                                    frame_num: 0,
                                    l: 0,
                                    m: 0.0,
                                    r: 0,
                                    pwr: 0.0,
                                    data: &header,
                                };
                                serde_cbor::to_vec(&pkt).ok()
                            }) {
                                Some(v) => v,
                                None => continue,
                            };

                        let next_basic_info = state.basic_info_json(next_id.as_str()).await;

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
                                header_pkt: next_header_pkt,
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

    #[test]
    fn scaled_relative_variance_power_is_zero_for_empty_or_dc() {
        assert_eq!(scaled_relative_variance_power(&[]), 0.0);
        assert_eq!(
            scaled_relative_variance_power(&[Complex32::new(1.0, 0.0)]),
            0.0
        );
        let bins = vec![Complex32::new(2.0, 0.0); 128];
        let scaled = scaled_relative_variance_power(&bins);
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
            assert!(s.update(false, v));
        }
    }

    #[test]
    fn squelch_closes_after_sustained_low_variation() {
        let mut s = SquelchState::new();
        assert!(s.update(true, 20.0), "strong variation should open squelch");
        for _ in 0..9 {
            assert!(
                s.update(true, 0.0),
                "should remain open until close hysteresis triggers"
            );
        }
        assert!(
            !s.update(true, 0.0),
            "should close after sustained low variance"
        );
    }

    #[test]
    fn squelch_opens_immediately_on_strong_variation() {
        let mut s = SquelchState::new();
        assert!(!s.update(true, 0.0));
        assert!(s.update(true, 100.0));
    }
}

pub struct AudioPipeline {
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
    pcm_frame_i32: Vec<i32>,
    pcm_frame_i16: Vec<i16>,
    pcm_accum: Vec<i32>,
    pcm_accum_offset: usize,
    flac_block_size: usize,
    flac_pwr_sum: f32,
    flac_pwr_frames: usize,
    dc: DcBlocker,
    agc: Agc,
    fm_prev: Complex32,
    pub flac: FlacStreamEncoder,
    last_agc: (AgcSpeed, Option<f32>, Option<f32>),
    squelch: SquelchState,
}

impl AudioPipeline {
    pub fn new(sample_rate: usize, audio_fft_size: usize) -> anyhow::Result<Self> {
        let mut planner = FftPlanner::<f32>::new();
        let ifft = planner.plan_fft_inverse(audio_fft_size);

        let mut real_planner = RealFftPlanner::<f32>::new();
        let c2r_ifft = real_planner.plan_fft_inverse(audio_fft_size);
        let c2r_scratch = c2r_ifft.make_scratch_vec();

        let frame_samples = audio_fft_size / 2;

        // Encode a slightly larger FLAC block than the per-FFT frame output.
        // This reduces websocket packet rate and browser-side scheduling overhead without
        // changing the DSP/FFT configuration or the FLAC stream format.
        let target_block_sec = 0.020_f64; // ~20ms
        let min_block = ((sample_rate as f64) * target_block_sec).ceil().max(1.0) as usize;
        let mut flac_block_size = frame_samples.max(min_block);
        flac_block_size = flac_block_size.div_ceil(8) * 8; // keep alignment friendly
        flac_block_size = flac_block_size.clamp(frame_samples, 8192);

        let flac = FlacStreamEncoder::new(sample_rate, 16, flac_block_size)?;

        Ok(Self {
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
            pcm_frame_i32: vec![0; frame_samples],
            pcm_frame_i16: vec![0; frame_samples],
            pcm_accum: Vec::with_capacity(flac_block_size * 4),
            pcm_accum_offset: 0,
            flac_block_size,
            flac_pwr_sum: 0.0,
            flac_pwr_frames: 0,
            // Keep the DC blocker cutoff low so AM has real low end; bass boost is frontend-only.
            dc: DcBlocker::new((sample_rate / 20).max(128)),
            // Match reference defaults.
            agc: Agc::new(0.1, 100.0, 30.0, 100.0, sample_rate as f32),
            fm_prev: Complex32::new(0.0, 0.0),
            flac,
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
        self.pcm_accum.clear();
        self.pcm_accum_offset = 0;
        self.flac_pwr_sum = 0.0;
        self.flac_pwr_frames = 0;
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

        let scaled_rv = scaled_relative_variance_power(spectrum_slice);
        let squelch_open = self.squelch.update(params.squelch_enabled, scaled_rv);
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

                self.carrier.copy_from_slice(&self.buf_in);
                let cutoff =
                    (500 * self.audio_fft_size / self.audio_rate).min(self.audio_fft_size / 2);
                for i in cutoff..(self.audio_fft_size - cutoff) {
                    self.carrier[i] = Complex32::new(0.0, 0.0);
                }
                self.ifft
                    .process_with_scratch(&mut self.carrier, &mut self.scratch);

                if frame_num % 2 == 1
                    && (((audio_mid_idx % 2 == 0) && !is_real_input)
                        || ((audio_mid_idx % 2 != 0) && is_real_input))
                {
                    negate_complex(&mut self.baseband);
                    negate_complex(&mut self.carrier);
                }

                add_complex(
                    &mut self.baseband[..self.audio_fft_size / 2],
                    &self.baseband_prev,
                );
                add_complex(
                    &mut self.carrier[..self.audio_fft_size / 2],
                    &self.carrier_prev,
                );

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
        self.carrier_prev
            .copy_from_slice(&self.carrier[self.audio_fft_size / 2..]);

        self.apply_agc_settings(params);

        let half = self.audio_fft_size / 2;
        let audio_out = &mut self.real[..half];
        self.dc.remove_dc(audio_out);
        self.agc.process(audio_out);

        // Match stream format: FLAC bits_per_sample=16.
        float_to_i16_centered(audio_out, &mut self.pcm_frame_i16, 32768.0);
        for (dst, src) in self.pcm_frame_i32.iter_mut().zip(self.pcm_frame_i16.iter()) {
            *dst = *src as i32;
        }

        // Accumulate a few frames worth of PCM before encoding a FLAC block.
        // This reduces packet rate without changing the websocket framing format.
        self.pcm_accum.extend_from_slice(&self.pcm_frame_i32);
        self.flac_pwr_sum += spectrum_slice.iter().map(|c| c.norm_sqr()).sum::<f32>();
        self.flac_pwr_frames += 1;

        // Drain complete FLAC blocks.
        loop {
            let available = self.pcm_accum.len().saturating_sub(self.pcm_accum_offset);
            if available < self.flac_block_size {
                break;
            }
            let end = self.pcm_accum_offset + self.flac_block_size;
            let block = &self.pcm_accum[self.pcm_accum_offset..end];
            let flac_bytes = self.flac.encode_block(block)?;
            self.pcm_accum_offset = end;

            // Compact occasionally to avoid unbounded growth.
            if self.pcm_accum_offset >= self.flac_block_size * 4 {
                self.pcm_accum.drain(0..self.pcm_accum_offset);
                self.pcm_accum_offset = 0;
            }

            let frames = self.flac_pwr_frames.max(1) as f32;
            let pwr = self.flac_pwr_sum / frames;
            self.flac_pwr_sum = 0.0;
            self.flac_pwr_frames = 0;

            let pkt = AudioPacket {
                frame_num,
                l: 0,
                m: params.m,
                r: spectrum_slice.len() as i32,
                pwr,
                data: &flac_bytes,
            };
            out_packets.push(serde_cbor::to_vec(&pkt)?);
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
        let scaled = scaled_relative_variance_power(&bins);
        assert!(scaled.abs() < 1e-4, "expected scaled near 0, got {scaled}");
    }

    #[test]
    fn scaled_relative_variance_is_large_for_single_bin_spike() {
        // For N bins, powers [1, 0, 0, ...] yields rv = N-1 and scaled = (N-2)*sqrt(N).
        let mut bins = vec![Complex32::new(0.0, 0.0); 64];
        bins[0] = Complex32::new(1.0, 0.0);
        let scaled = scaled_relative_variance_power(&bins);
        assert!(
            scaled > 100.0,
            "expected scaled to be large for a single-bin spike, got {scaled}"
        );
    }

    #[test]
    fn squelch_state_machine_opens_on_consecutive_soft_hits_and_closes_with_hysteresis() {
        let mut s = SquelchState::new();

        // Enabling squelch closes it until a signal is detected.
        assert!(
            !s.update(true, 0.0),
            "expected closed immediately after enable"
        );

        // Soft open: scaled >= 5 for 3 consecutive frames.
        assert!(!s.update(true, 6.0));
        assert!(!s.update(true, 6.0));
        assert!(
            s.update(true, 6.0),
            "expected open after 3 consecutive soft hits"
        );

        // Close hysteresis: scaled < 2 for 10 consecutive frames.
        for _ in 0..9 {
            assert!(
                s.update(true, 1.0),
                "expected to remain open during close hysteresis"
            );
        }
        assert!(
            !s.update(true, 1.0),
            "expected to close after hysteresis completes"
        );
    }
}
