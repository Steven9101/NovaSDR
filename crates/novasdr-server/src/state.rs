use anyhow::{anyhow, Context};
use axum::{extract::State, response::IntoResponse, Json};
use dashmap::DashMap;
use novasdr_core::{
    config,
    protocol::{json_stringify_value, EventsInfo},
};
use serde_json::json;
use std::{
    collections::HashMap,
    net::IpAddr,
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, RwLock};
use tracing::warn;

// Audio packets can be bursty (GC pauses, GPU sync, OS scheduler jitter). A slightly deeper queue
// smooths transient stalls without changing steady-state throughput.
const AUDIO_QUEUE_CAPACITY: usize = 128;
const WATERFALL_QUEUE_CAPACITY: usize = 8;
const TEXT_QUEUE_CAPACITY: usize = 64;

pub type ClientId = u64;

pub struct ReceiverState {
    pub receiver: config::ReceiverConfig,
    pub rt: Arc<config::Runtime>,
    pub audio_clients: DashMap<ClientId, Arc<AudioClient>>,
    pub waterfall_clients: Vec<DashMap<ClientId, Arc<WaterfallClient>>>,
    pub signal_changes: DashMap<String, (i32, f64, i32)>,
}

impl ReceiverState {
    pub fn new(receiver: config::ReceiverConfig, rt: Arc<config::Runtime>) -> Self {
        let mut waterfall_clients = Vec::with_capacity(rt.downsample_levels);
        for _ in 0..rt.downsample_levels {
            waterfall_clients.push(DashMap::new());
        }

        Self {
            receiver,
            rt,
            audio_clients: DashMap::new(),
            waterfall_clients,
            signal_changes: DashMap::new(),
        }
    }
}

pub struct AppState {
    pub cfg: Arc<config::Config>,
    pub html_root: std::path::PathBuf,
    pub receivers: HashMap<String, Arc<ReceiverState>>,
    pub active_receiver: Arc<ReceiverState>,
    pub markers: Arc<RwLock<serde_json::Value>>,
    pub bands: Arc<RwLock<serde_json::Value>>,

    pub event_clients: DashMap<ClientId, mpsc::Sender<Arc<str>>>,
    pub chat_clients: DashMap<ClientId, mpsc::Sender<Arc<str>>>,
    pub chat_history: tokio::sync::Mutex<Vec<ChatMessage>>,
    ws_ip_counts: DashMap<IpAddr, usize>,

    pub total_waterfall_bits: AtomicUsize,
    pub total_audio_bits: AtomicUsize,
    pub waterfall_kbits_per_sec: AtomicU64,
    pub audio_kbits_per_sec: AtomicU64,
    pub dropped_waterfall_frames: AtomicU64,
    pub dropped_audio_frames: AtomicU64,

    pub next_client_id: AtomicU64,
}

impl AppState {
    pub fn new(cfg: Arc<config::Config>, html_root: std::path::PathBuf) -> anyhow::Result<Self> {
        let mut receivers = HashMap::new();
        for r in cfg.receivers.iter() {
            let rt = Arc::new(
                cfg.runtime_for(r.id.as_str())
                    .with_context(|| format!("derive runtime for receiver {}", r.id))?,
            );
            receivers.insert(r.id.clone(), Arc::new(ReceiverState::new(r.clone(), rt)));
        }

        let active_receiver = receivers
            .get(cfg.active_receiver_id.as_str())
            .cloned()
            .ok_or_else(|| anyhow!("active_receiver_id missing from receiver map"))?;

        Ok(Self {
            cfg,
            html_root,
            receivers,
            active_receiver,
            markers: Arc::new(RwLock::new(serde_json::Value::Null)),
            bands: Arc::new(RwLock::new(serde_json::Value::Null)),
            event_clients: DashMap::new(),
            chat_clients: DashMap::new(),
            chat_history: tokio::sync::Mutex::new(load_chat_history()),
            ws_ip_counts: DashMap::new(),
            total_waterfall_bits: AtomicUsize::new(0),
            total_audio_bits: AtomicUsize::new(0),
            waterfall_kbits_per_sec: AtomicU64::new(0),
            audio_kbits_per_sec: AtomicU64::new(0),
            dropped_waterfall_frames: AtomicU64::new(0),
            dropped_audio_frames: AtomicU64::new(0),
            next_client_id: AtomicU64::new(1),
        })
    }

    pub fn alloc_client_id(&self) -> ClientId {
        self.next_client_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn receiver_state(&self, receiver_id: &str) -> Option<&Arc<ReceiverState>> {
        self.receivers.get(receiver_id)
    }

    pub fn active_receiver_id(&self) -> &str {
        self.cfg.active_receiver_id.as_str()
    }

    pub fn active_receiver_state(&self) -> &Arc<ReceiverState> {
        &self.active_receiver
    }

    pub fn total_audio_clients(&self) -> usize {
        self.receivers
            .values()
            .map(|r| r.audio_clients.len())
            .sum::<usize>()
    }

    pub fn total_waterfall_clients(&self) -> usize {
        self.receivers
            .values()
            .map(|r| r.waterfall_clients.iter().map(|m| m.len()).sum::<usize>())
            .sum::<usize>()
    }

    pub fn try_acquire_ws_ip(self: &Arc<Self>, ip: IpAddr) -> Option<WsIpGuard> {
        let limit = self.cfg.limits.ws_per_ip.max(1);
        let mut entry = self.ws_ip_counts.entry(ip).or_insert(0);
        if *entry >= limit {
            return None;
        }
        *entry += 1;
        Some(WsIpGuard {
            state: self.clone(),
            ip,
        })
    }

    fn release_ws_ip(&self, ip: IpAddr) {
        if let Some(mut entry) = self.ws_ip_counts.get_mut(&ip) {
            if *entry > 1 {
                *entry -= 1;
                return;
            }
        }
        self.ws_ip_counts.remove(&ip);
    }

    pub async fn basic_info_json(&self, receiver_id: &str) -> String {
        let Some(receiver) = self.receiver_state(receiver_id) else {
            return "{}".to_string();
        };
        let grid_locator = self.cfg.websdr.grid_locator.clone();
        let markers = self.markers.read().await;
        let markers_str = json_stringify_value(&markers);
        let bands = self.bands.read().await;
        let bands_str = json_stringify_value(&bands);

        let ssb_lowcut_hz = receiver
            .receiver
            .input
            .defaults
            .ssb_lowcut_hz
            .unwrap_or(100)
            .max(0);
        let ssb_highcut_hz = receiver
            .receiver
            .input
            .defaults
            .ssb_highcut_hz
            .unwrap_or(2800)
            .max(ssb_lowcut_hz.saturating_add(1));

        let defaults = json!({
            "frequency": receiver.rt.default_frequency,
            "modulation": receiver.rt.default_mode_str,
            "l": receiver.rt.default_l,
            "m": receiver.rt.default_m,
            "r": receiver.rt.default_r,
            "ssb_lowcut_hz": ssb_lowcut_hz,
            "ssb_highcut_hz": ssb_highcut_hz,
            "squelch_enabled": receiver.receiver.input.defaults.squelch_enabled,
        });

        let out = json!({
            "receiver_id": receiver.receiver.id,
            "receiver_name": receiver.receiver.name,
            "sps": receiver.rt.sps,
            "audio_max_sps": receiver.rt.audio_max_sps,
            "audio_max_fft": receiver.rt.audio_max_fft_size,
            "fft_size": receiver.rt.fft_size,
            "fft_result_size": receiver.rt.fft_result_size,
            "waterfall_size": receiver.rt.min_waterfall_fft,
            "basefreq": receiver.rt.basefreq,
            "total_bandwidth": receiver.rt.total_bandwidth,
            "overlap": receiver.rt.fft_size / 2,
            "fft_overlap": receiver.rt.fft_size / 2,
            "defaults": defaults,
            "waterfall_compression": receiver.rt.waterfall_compression_str,
            "audio_compression": receiver.rt.audio_compression_str,
            "grid_locator": grid_locator,
            "smeter_offset": receiver.receiver.input.smeter_offset,
            "markers": markers_str,
            "bands": bands_str,
        });

        match serde_json::to_string(&out) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = ?e, "failed to serialize receiver basic info");
                "{}".to_string()
            }
        }
    }

    pub fn broadcast_signal_changes(
        &self,
        receiver_id: &str,
        unique_id: &str,
        l: i32,
        m: f64,
        r: i32,
    ) {
        let Some(receiver) = self.receiver_state(receiver_id) else {
            return;
        };
        if !receiver.rt.show_other_users {
            return;
        }
        if l == -1 && r == -1 {
            receiver.signal_changes.remove(unique_id);
        } else {
            receiver
                .signal_changes
                .insert(unique_id.to_string(), (l, m, r));
        }
    }

    pub fn event_info(&self, include_changes: bool) -> EventsInfo {
        let waterfall_clients = self.total_waterfall_clients();
        let signal_clients = self.total_audio_clients();

        let show_other_users = self.cfg.server.otherusers > 0;
        let signal_changes = if include_changes && show_other_users {
            let mut map = HashMap::new();
            for (rx_id, rx) in self.receivers.iter() {
                for entry in rx.signal_changes.iter() {
                    map.insert(format!("{rx_id}:{}", entry.key()), *entry.value());
                }
            }
            Some(map)
        } else {
            None
        };

        EventsInfo {
            waterfall_clients,
            signal_clients,
            signal_changes,
            waterfall_kbits: (self.waterfall_kbits_per_sec.load(Ordering::Relaxed) as f64) / 1.0,
            audio_kbits: (self.audio_kbits_per_sec.load(Ordering::Relaxed) as f64) / 1.0,
        }
    }
}

pub struct WsIpGuard {
    state: Arc<AppState>,
    ip: IpAddr,
}

impl Drop for WsIpGuard {
    fn drop(&mut self) {
        self.state.release_ws_ip(self.ip);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub username: String,
    pub message: String,
    pub timestamp: String,
    pub user_id: String,
    pub r#type: String,
    #[serde(default)]
    pub reply_to_id: String,
    #[serde(default)]
    pub reply_to_username: String,
}

fn load_chat_history() -> Vec<ChatMessage> {
    let path = Path::new("chat_history.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = ?e, path = %path.display(), "failed to parse chat history; starting empty");
            Vec::new()
        }
    }
}

pub async fn append_chat_message(state: &AppState, msg: ChatMessage) {
    let mut hist = state.chat_history.lock().await;
    hist.push(msg);
    if hist.len() > 20 {
        let overflow = hist.len() - 20;
        hist.drain(0..overflow);
    }
    if let Ok(raw) = serde_json::to_string(&*hist) {
        if let Err(e) = tokio::fs::write("chat_history.json", raw).await {
            warn!(error = ?e, path = "chat_history.json", "failed to persist chat history");
        }
    }
}

pub struct AudioClient {
    pub unique_id: String,
    pub tx: mpsc::Sender<Vec<u8>>,
    pub params: std::sync::Mutex<AudioParams>,
    pub pipeline: std::sync::Mutex<crate::ws::audio::AudioPipeline>,
}

#[derive(Debug, Clone)]
pub struct AudioParams {
    pub l: i32,
    pub m: f64,
    pub r: i32,
    pub mute: bool,
    pub squelch_enabled: bool,
    pub demodulation: novasdr_core::dsp::demod::DemodulationMode,
    pub agc_speed: AgcSpeed,
    pub agc_attack_ms: Option<f32>,
    pub agc_release_ms: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgcSpeed {
    Default,
    Off,
    Fast,
    Medium,
    Slow,
    Custom,
}

impl AgcSpeed {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "off" => Self::Off,
            "fast" => Self::Fast,
            "medium" => Self::Medium,
            "slow" => Self::Slow,
            "custom" => Self::Custom,
            _ => Self::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agc_speed_parse_maps_known_values_and_defaults() {
        assert_eq!(AgcSpeed::parse("off"), AgcSpeed::Off);
        assert_eq!(AgcSpeed::parse("fast"), AgcSpeed::Fast);
        assert_eq!(AgcSpeed::parse("medium"), AgcSpeed::Medium);
        assert_eq!(AgcSpeed::parse("slow"), AgcSpeed::Slow);
        assert_eq!(AgcSpeed::parse("custom"), AgcSpeed::Custom);
        assert_eq!(AgcSpeed::parse("default"), AgcSpeed::Default);
        assert_eq!(AgcSpeed::parse(""), AgcSpeed::Default);
        assert_eq!(AgcSpeed::parse("???"), AgcSpeed::Default);
    }
}

pub struct WaterfallClient {
    pub tx: mpsc::Sender<WaterfallWorkItem>,
    pub params: std::sync::Mutex<WaterfallParams>,
}

pub fn audio_channel() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
    mpsc::channel(AUDIO_QUEUE_CAPACITY)
}

#[derive(Debug, Clone)]
pub struct WaterfallWorkItem {
    pub frame_num: u64,
    pub level: usize,
    pub l: usize,
    pub r: usize,
    pub quantized_concat: Arc<[i8]>,
    pub quantized_offset: usize,
}

pub fn waterfall_channel() -> (
    mpsc::Sender<WaterfallWorkItem>,
    mpsc::Receiver<WaterfallWorkItem>,
) {
    mpsc::channel(WATERFALL_QUEUE_CAPACITY)
}

pub fn text_channel() -> (mpsc::Sender<Arc<str>>, mpsc::Receiver<Arc<str>>) {
    mpsc::channel(TEXT_QUEUE_CAPACITY)
}

#[derive(Debug, Clone)]
pub struct WaterfallParams {
    pub level: usize,
    pub l: usize,
    pub r: usize,
}

pub async fn server_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = &state.cfg;
    Json(json!({
        "serverName": cfg.websdr.name,
        "location": cfg.websdr.grid_locator,
        "operators": [{ "name": cfg.websdr.operator }],
        "email": cfg.websdr.email,
        "chatEnabled": cfg.websdr.chat_enabled,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn receivers_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = &state.cfg;
    let receivers = cfg
        .receivers
        .iter()
        .map(|r| {
            let rt = state
                .receiver_state(r.id.as_str())
                .map(|rx| rx.rt.as_ref())
                .map(|rt| (rt.basefreq, rt.basefreq + rt.total_bandwidth));
            json!({
                "id": r.id,
                "name": r.name,
                "driver": r.input.driver.as_str(),
                "min_hz": rt.map(|(min, _)| min),
                "max_hz": rt.map(|(_, max)| max),
            })
        })
        .collect::<Vec<_>>();
    Json(json!({
        "active_receiver_id": cfg.active_receiver_id,
        "receivers": receivers,
    }))
}

async fn maybe_load_json(path: &Path) -> Option<serde_json::Value> {
    let raw = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str::<serde_json::Value>(&raw).ok()
}

pub async fn load_overlays_once(state: Arc<AppState>, overlays_dir: std::path::PathBuf) {
    let markers_path = overlays_dir.join("markers.json");
    if let Some(v) = maybe_load_json(&markers_path).await {
        let mut cur = state.markers.write().await;
        *cur = v;
    }

    let bands_path = overlays_dir.join("bands.json");
    if let Some(v) = maybe_load_json(&bands_path).await {
        let mut cur = state.bands.write().await;
        *cur = v;
    }
}

pub fn spawn_marker_watcher(state: Arc<AppState>, overlays_dir: std::path::PathBuf) {
    tokio::spawn(async move {
        loop {
            let path = overlays_dir.join("markers.json");
            if let Some(v) = maybe_load_json(&path).await {
                let mut cur = state.markers.write().await;
                if *cur != v {
                    *cur = v;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

pub fn spawn_bands_watcher(state: Arc<AppState>, overlays_dir: std::path::PathBuf) {
    tokio::spawn(async move {
        loop {
            let path = overlays_dir.join("bands.json");
            if let Some(v) = maybe_load_json(&path).await {
                let mut cur = state.bands.write().await;
                if *cur != v {
                    *cur = v;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
