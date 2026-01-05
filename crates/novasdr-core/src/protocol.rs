use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct BasicInfoDefaults {
    pub frequency: i64,
    pub modulation: String,
    pub l: i32,
    pub m: f64,
    pub r: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssb_lowcut_hz: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssb_highcut_hz: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squelch_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BasicInfo {
    pub sps: i64,
    pub audio_max_sps: i64,
    pub audio_max_fft: usize,
    pub fft_size: usize,
    pub fft_result_size: usize,
    pub waterfall_size: usize,
    pub basefreq: i64,
    pub total_bandwidth: i64,
    pub defaults: BasicInfoDefaults,
    pub waterfall_compression: String,
    pub audio_compression: String,
    pub grid_locator: String,
    pub smeter_offset: i32,
    pub markers: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum ClientCommand {
    Receiver {
        receiver_id: String,
    },
    Window {
        l: i32,
        r: i32,
        #[serde(default)]
        m: Option<f64>,
        #[serde(default)]
        level: Option<i32>,
    },
    Demodulation {
        demodulation: String,
    },
    Userid {
        userid: String,
    },
    Mute {
        mute: bool,
    },
    Squelch {
        enabled: bool,
    },
    Chat {
        message: String,
        username: String,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        reply_to_id: Option<String>,
        #[serde(default)]
        reply_to_username: Option<String>,
    },
    Agc {
        speed: String,
        #[serde(default)]
        attack: Option<f32>,
        #[serde(default)]
        release: Option<f32>,
    },
    Buffer {
        size: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EventsInfo {
    pub waterfall_clients: usize,
    pub signal_clients: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_changes: Option<std::collections::HashMap<String, (i32, f64, i32)>>,
    pub waterfall_kbits: f64,
    pub audio_kbits: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioPacket<'a> {
    pub frame_num: u64,
    pub l: i32,
    pub m: f64,
    pub r: i32,
    pub pwr: f32,
    #[serde(with = "serde_bytes")]
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterfallPacket<'a> {
    pub frame_num: u64,
    pub l: i32,
    pub r: i32,
    #[serde(with = "serde_bytes")]
    pub data: &'a [u8],
}

pub fn json_stringify_markers(markers: &Value) -> String {
    json_stringify_value(markers)
}

pub fn json_stringify_value(v: &Value) -> String {
    match serde_json::to_string(v) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, "failed to serialize json value");
            "{}".to_string()
        }
    }
}
