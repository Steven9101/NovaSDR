use anyhow::Context;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

#[derive(Debug, Clone)]
pub struct Config {
    pub server: Server,
    pub websdr: WebSdr,
    pub limits: Limits,
    pub receivers: Vec<ReceiverConfig>,
    pub active_receiver_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_html_root")]
    pub html_root: String,
    #[serde(default)]
    pub otherusers: i64,
    #[serde(default = "default_threads")]
    pub threads: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebSdr {
    #[serde(default)]
    pub register_online: bool,
    #[serde(default = "default_sdr_list_url")]
    pub register_url: String,
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub antenna: String,
    #[serde(default = "default_grid")]
    pub grid_locator: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub operator: String,
    #[serde(default)]
    pub email: String,
    #[serde(default = "default_callsign_lookup")]
    pub callsign_lookup_url: String,
    #[serde(default = "default_chat_enabled")]
    pub chat_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limits {
    #[serde(default = "default_limit")]
    pub audio: usize,
    #[serde(default = "default_limit")]
    pub waterfall: usize,
    #[serde(default = "default_limit")]
    pub events: usize,
    #[serde(default = "default_ws_per_ip")]
    pub ws_per_ip: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiverConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub input: ReceiverInput,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiverInput {
    pub sps: i64,
    pub frequency: i64,
    pub signal: SignalType,
    #[serde(default = "default_fft_size")]
    pub fft_size: usize,
    #[serde(default)]
    pub brightness_offset: i32,
    #[serde(default = "default_audio_sps")]
    pub audio_sps: i64,
    #[serde(default = "default_waterfall_size")]
    pub waterfall_size: usize,
    #[serde(default = "default_waterfall_compression")]
    pub waterfall_compression: WaterfallCompression,
    #[serde(default = "default_audio_compression")]
    pub audio_compression: AudioCompression,
    #[serde(default)]
    pub smeter_offset: i32,
    #[serde(default)]
    pub accelerator: Accelerator,
    pub driver: InputDriver,
    #[serde(default)]
    pub defaults: ReceiverDefaults,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiverDefaults {
    #[serde(default = "default_default_frequency")]
    pub frequency: i64,
    #[serde(default = "default_default_modulation")]
    pub modulation: String,
    /// Optional SSB default passband edges in Hz (positive values).
    ///
    /// When `modulation` is `USB`, the default audio window is `+lowcut..+highcut`.
    /// When `modulation` is `LSB`, the default audio window is `-highcut..-lowcut`.
    #[serde(default)]
    pub ssb_lowcut_hz: Option<i64>,
    #[serde(default)]
    pub ssb_highcut_hz: Option<i64>,
    #[serde(default)]
    pub squelch_enabled: bool,
}

impl Default for ReceiverDefaults {
    fn default() -> Self {
        Self {
            frequency: default_default_frequency(),
            modulation: default_default_modulation(),
            ssb_lowcut_hz: None,
            ssb_highcut_hz: None,
            squelch_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub enum InputDriver {
    #[serde(rename = "stdin")]
    Stdin { format: SampleFormat },
    #[serde(rename = "soapysdr")]
    SoapySdr(SoapySdrDriver),
}

#[derive(Debug, Clone, Deserialize)]
pub struct SoapySdrDriver {
    pub device: String,
    #[serde(default)]
    pub channel: usize,
    #[serde(default)]
    pub antenna: Option<String>,
    pub format: SampleFormat,
    #[serde(default)]
    pub agc: Option<bool>,
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(default)]
    pub gains: BTreeMap<String, f64>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub stream_args: BTreeMap<String, String>,
    #[serde(default = "default_soapysdr_rx_buffer_samples")]
    pub rx_buffer_samples: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SignalType {
    Real,
    Iq,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WaterfallCompression {
    Zstd,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioCompression {
    Flac,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Accelerator {
    #[default]
    None,
    Clfft,
    Vkfft,
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SampleFormat {
    U8,
    S8,
    U16,
    S16,
    Cs16,
    F32,
    Cf32,
    F64,
}

fn default_port() -> u16 {
    9002
}
fn default_host() -> String {
    "[::]".to_string()
}
fn default_html_root() -> String {
    "frontend/dist/".to_string()
}
fn default_threads() -> usize {
    0
}
fn default_soapysdr_rx_buffer_samples() -> usize {
    65536
}
fn default_name() -> String {
    "NovaSDR".to_string()
}
fn default_grid() -> String {
    "-".to_string()
}
fn default_callsign_lookup() -> String {
    "https://www.qrz.com/db/".to_string()
}
fn default_sdr_list_url() -> String {
    "https://sdr-list.xyz/api/update_websdr".to_string()
}
fn default_chat_enabled() -> bool {
    true
}
fn default_limit() -> usize {
    1000
}
fn default_ws_per_ip() -> usize {
    50
}
fn default_fft_size() -> usize {
    131_072
}
fn default_audio_sps() -> i64 {
    12_000
}
fn default_waterfall_size() -> usize {
    1024
}
fn default_waterfall_compression() -> WaterfallCompression {
    WaterfallCompression::Zstd
}
fn default_audio_compression() -> AudioCompression {
    AudioCompression::Flac
}
fn default_default_frequency() -> i64 {
    -1
}
fn default_default_modulation() -> String {
    "USB".to_string()
}

impl Default for Server {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            html_root: default_html_root(),
            otherusers: 1,
            threads: default_threads(),
        }
    }
}

impl Default for WebSdr {
    fn default() -> Self {
        Self {
            register_online: false,
            register_url: default_sdr_list_url(),
            name: default_name(),
            antenna: String::new(),
            grid_locator: default_grid(),
            hostname: String::new(),
            operator: String::new(),
            email: String::new(),
            callsign_lookup_url: default_callsign_lookup(),
            chat_enabled: default_chat_enabled(),
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            audio: default_limit(),
            waterfall: default_limit(),
            events: default_limit(),
            ws_per_ip: default_ws_per_ip(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GlobalConfigFile {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub websdr: WebSdr,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub active_receiver_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReceiversFile {
    pub receivers: Vec<ReceiverConfig>,
}

pub fn load_from_files(config_json: &Path, receivers_json: &Path) -> anyhow::Result<Config> {
    let raw = std::fs::read_to_string(config_json)
        .with_context(|| format!("read {}", config_json.display()))?;
    let global: GlobalConfigFile =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_json.display()))?;

    let raw = std::fs::read_to_string(receivers_json)
        .with_context(|| format!("read {}", receivers_json.display()))?;
    let mut receivers: ReceiversFile = serde_json::from_str(&raw)
        .with_context(|| format!("parse {}", receivers_json.display()))?;

    anyhow::ensure!(
        !receivers.receivers.is_empty(),
        "receivers.json must contain at least one receiver"
    );

    let mut ids = HashSet::<String>::new();
    let mut stdin_receivers = 0usize;
    for r in receivers.receivers.iter_mut() {
        let id_trimmed = r.id.trim();
        anyhow::ensure!(!id_trimmed.is_empty(), "receivers[].id must not be empty");
        if !ids.insert(r.id.clone()) {
            anyhow::bail!("duplicate receivers[].id {id_trimmed:?} in receivers.json");
        }
        if matches!(r.input.driver, InputDriver::Stdin { .. }) {
            stdin_receivers += 1;
        }
        if r.name.trim().is_empty() {
            r.name = r.id.clone();
        }
    }
    anyhow::ensure!(
        stdin_receivers <= 1,
        "only one receiver may use input.driver.kind = \"stdin\" (found {stdin_receivers})"
    );

    let active_id = match global.active_receiver_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ if receivers.receivers.len() == 1 => receivers.receivers[0].id.clone(),
        _ => anyhow::bail!(
            "active_receiver_id is required in config.json when receivers.json contains multiple receivers"
        ),
    };

    if !receivers.receivers.iter().any(|r| r.id == active_id) {
        anyhow::bail!("active_receiver_id {active_id:?} not found in receivers.json");
    }

    Ok(Config {
        server: global.server,
        websdr: global.websdr,
        limits: global.limits,
        receivers: receivers.receivers,
        active_receiver_id: active_id,
    })
}

#[derive(Debug, Clone)]
pub struct Runtime {
    pub sps: i64,
    pub fft_size: usize,
    pub fft_result_size: usize,
    pub is_real: bool,
    pub basefreq: i64,
    pub total_bandwidth: i64,
    pub downsample_levels: usize,
    pub audio_max_sps: i64,
    pub audio_max_fft_size: usize,
    pub min_waterfall_fft: usize,
    pub brightness_offset: i32,
    pub show_other_users: bool,
    pub default_frequency: i64,
    pub default_m: f64,
    pub default_l: i32,
    pub default_r: i32,
    pub default_mode_str: String,
    pub waterfall_compression_str: String,
    pub audio_compression_str: String,
}

impl Config {
    pub fn receiver(&self, receiver_id: &str) -> Option<&ReceiverConfig> {
        self.receivers.iter().find(|r| r.id == receiver_id)
    }

    pub fn active_receiver(&self) -> anyhow::Result<&ReceiverConfig> {
        self.receiver(self.active_receiver_id.as_str())
            .with_context(|| {
                format!(
                    "active_receiver_id {:?} not found in receivers",
                    self.active_receiver_id
                )
            })
    }

    pub fn runtime(&self) -> anyhow::Result<Runtime> {
        self.runtime_for(self.active_receiver_id.as_str())
    }

    pub fn runtime_for(&self, receiver_id: &str) -> anyhow::Result<Runtime> {
        let receiver = self
            .receiver(receiver_id)
            .with_context(|| format!("receiver {receiver_id:?} not found"))?;
        self.runtime_from_input(&receiver.input)
    }

    fn runtime_from_input(&self, input: &ReceiverInput) -> anyhow::Result<Runtime> {
        let sps = input.sps;
        anyhow::ensure!(sps > 0, "receiver.input.sps must be > 0");

        let fft_size = input.fft_size;
        anyhow::ensure!(
            fft_size.is_power_of_two(),
            "receiver.input.fft_size must be power of two"
        );

        let is_real = input.signal == SignalType::Real;
        let (fft_result_size, basefreq, total_bandwidth) = if is_real {
            (fft_size / 2, input.frequency, sps / 2)
        } else {
            (fft_size, input.frequency - sps / 2, sps)
        };

        let min_waterfall_fft = input.waterfall_size;
        let mut downsample_levels = 0usize;
        let mut cur = fft_result_size;
        while cur >= min_waterfall_fft {
            downsample_levels += 1;
            cur /= 2;
        }
        anyhow::ensure!(
            downsample_levels >= 1,
            "waterfall_size too large for fft_result_size"
        );

        let audio_max_sps = input.audio_sps;
        anyhow::ensure!(audio_max_sps > 0, "receiver.input.audio_sps must be > 0");
        let max_audio_sps = if is_real { sps / 2 } else { sps };
        anyhow::ensure!(
            audio_max_sps <= max_audio_sps,
            "receiver.input.audio_sps must be <= receiver input bandwidth ({max_audio_sps} Hz)"
        );
        anyhow::ensure!(
            audio_max_sps <= 48_000,
            "receiver.input.audio_sps must be <= 48000 Hz"
        );

        let audio_max_fft_size =
            ((((audio_max_sps as f64) * (fft_size as f64) / (sps as f64) / 4.0).ceil() as usize)
                * 4)
            .max(32);

        let show_other_users = self.server.otherusers > 0;

        let mut default_frequency = input.defaults.frequency;
        if default_frequency == -1 {
            default_frequency = basefreq + total_bandwidth / 2;
        }

        let mut default_m = if is_real {
            (default_frequency - basefreq) as f64 * (fft_result_size as f64) * 2.0 / (sps as f64)
        } else {
            (default_frequency - basefreq) as f64 * (fft_result_size as f64) / (sps as f64)
        };

        let offsets_3 = (3000_i64) * (fft_result_size as i64) / sps;
        let offsets_5 = (5000_i64) * (fft_result_size as i64) / sps;
        let offsets_96 = (96000_i64) * (fft_result_size as i64) / sps;

        let ssb_lowcut_hz = input.defaults.ssb_lowcut_hz.unwrap_or(300);
        let ssb_highcut_hz = input.defaults.ssb_highcut_hz.unwrap_or(3000);
        anyhow::ensure!(
            ssb_lowcut_hz >= 0,
            "receiver.input.defaults.ssb_lowcut_hz must be >= 0"
        );
        anyhow::ensure!(
            ssb_highcut_hz > ssb_lowcut_hz,
            "receiver.input.defaults.ssb_highcut_hz must be > receiver.input.defaults.ssb_lowcut_hz"
        );
        let offsets_ssb_low = ssb_lowcut_hz * (fft_result_size as i64) / sps;
        let offsets_ssb_high = ssb_highcut_hz * (fft_result_size as i64) / sps;

        let default_mode_str = input.defaults.modulation.to_uppercase();
        let (default_l, default_r) = match default_mode_str.as_str() {
            "LSB" => (
                (default_m as i64 - offsets_ssb_high) as i32,
                (default_m as i64 - offsets_ssb_low) as i32,
            ),
            "AM" | "SAM" | "FM" | "FMC" => (
                (default_m as i64 - offsets_5) as i32,
                (default_m as i64 + offsets_5) as i32,
            ),
            "WBFM" => (
                (default_m as i64 - offsets_96) as i32,
                (default_m as i64 + offsets_96) as i32,
            ),
            "USB" => (
                (default_m as i64 + offsets_ssb_low) as i32,
                (default_m as i64 + offsets_ssb_high) as i32,
            ),
            _ => (default_m as i32, (default_m as i64 + offsets_3) as i32),
        };

        default_m = default_m.clamp(0.0, fft_result_size as f64);
        let mut default_l = default_l.clamp(0, fft_result_size as i32);
        let mut default_r = default_r.clamp(0, fft_result_size as i32);

        let max_window = audio_max_fft_size.min(fft_result_size) as i32;
        if max_window > 0 && (default_r - default_l) > max_window {
            let center = default_m.round() as i32;
            let half = max_window / 2;
            default_l =
                (center - half).clamp(0, (fft_result_size as i32).saturating_sub(max_window));
            default_r = default_l + max_window;
        }

        let waterfall_compression_str = match input.waterfall_compression {
            WaterfallCompression::Zstd => "zstd".to_string(),
        };
        let audio_compression_str = match input.audio_compression {
            AudioCompression::Flac => "flac".to_string(),
        };

        Ok(Runtime {
            sps,
            fft_size,
            fft_result_size,
            is_real,
            basefreq,
            total_bandwidth,
            downsample_levels,
            audio_max_sps,
            audio_max_fft_size,
            min_waterfall_fft,
            brightness_offset: input.brightness_offset,
            show_other_users,
            default_frequency,
            default_m,
            default_l,
            default_r,
            default_mode_str,
            waterfall_compression_str,
            audio_compression_str,
        })
    }
}
