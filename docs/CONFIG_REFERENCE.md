# Configuration Reference

NovaSDR uses **two JSON files**:

- `config/config.json`: global server/WebSDR settings
- `config/receivers.json`: receiver-specific DSP + input settings

NovaSDR also optionally reads additional JSON files from `config/overlays/` next to `config/config.json`.
If the files are missing, NovaSDR creates defaults on startup (empty markers; a basic band plan).

- `config/overlays/markers.json`: static frequency markers shown in the UI
- `config/overlays/bands.json`: band plan overlays and "jump to band" entries

## `config/config.json`

### `server`

| Key | Type | Default | Notes |
|---|---:|---:|---|
| `port` | int | `9002` | Listen port |
| `host` | string | `"0.0.0.0"` | Bind address |
| `html_root` | string | `"html/"` | Static UI directory (e.g. `frontend/dist/`) |
| `otherusers` | int | `1` | Enables "other users" overlays (`/events` `signal_changes`) |
| `threads` | int | `0` | Tokio worker thread count (`0` = auto; clamped to available CPU cores) |

### `websdr`

| Key | Type | Default | Notes |
|---|---:|---:|---|
| `register_online` | bool | `false` | Enables periodic registration updates to `register_url` |
| `register_url` | string | `"https://sdr-list.xyz/api/update_websdr"` | Registration endpoint |
| `name` | string | `"NovaSDR"` | Used by `/server-info.json` |
| `antenna` | string | `""` | Informational |
| `grid_locator` | string | `"-"` | Used by UI and settings |
| `hostname` | string | `""` | Informational |
| `operator` | string | `""` | Used by `/server-info.json` |
| `email` | string | `""` | Used by `/server-info.json` |
| `callsign_lookup_url` | string | `"https://www.qrz.com/db/"` | UI link |
| `chat_enabled` | bool | `true` | Enables chat in UI |

### `limits`

Enforced at WebSocket connection time. When the limit is reached, new connections are rejected with HTTP `429`.

| Key | Type | Default |
|---|---:|---:|
| `audio` | int | `1000` |
| `waterfall` | int | `1000` |
| `events` | int | `1000` |
| `ws_per_ip` | int | `50` |

### `active_receiver_id`

| Key | Type | Default | Notes |
|---|---:|---:|---|
| `active_receiver_id` | string | (required if multiple receivers) | Selects a receiver by `id` from `config/receivers.json` |

## `config/receivers.json`

Top-level:

| Key | Type | Notes |
|---|---:|---|
| `receivers` | array | One or more receivers |

Each entry in `receivers[]`:

| Key | Type | Notes |
|---|---:|---|
| `id` | string | Unique identifier (must match `active_receiver_id`) |
| `name` | string | Display name (defaults to `id` if empty) |
| `input` | object | Receiver DSP + input settings |

### `receivers[].input`

| Key | Type | Required | Notes |
|---|---:|---:|---|
| `sps` | int | yes | Input sample rate (samples/sec) |
| `frequency` | int | yes | Center frequency (Hz) |
| `signal` | `"iq"` \| `"real"` | yes | Determines FFT layout |
| `fft_size` | int | no | Must be power-of-two for the FFT engine |
| `brightness_offset` | int | no | Waterfall visual offset |
| `audio_sps` | int | no | Target audio passband rate; used to derive `audio_max_fft_size` and limits how wide the tuned audio window can be. Must be `<= 48000`. The backend FLAC stream uses this sample rate; the browser resamples for playback and caps output to 48 kHz. |
| `waterfall_size` | int | no | Target waterfall width at client; drives downsample level selection |
| `waterfall_compression` | `"zstd"` | no | Only `zstd` supported |
| `audio_compression` | `"flac"` | no | Only `flac` supported |
| `accelerator` | `"none"` \| `"clfft"` \| `"vkfft"` | no | `clfft` requires building with `--features clfft`; `vkfft` requires building with `--features vkfft` |
| `smeter_offset` | int | no | UI-only offset |

### `receivers[].input.driver`

This is a tagged union with a `kind` discriminator:

- `{"kind": "stdin", "format": "u8"}`
- `{"kind": "soapysdr", "device": "...", "format": "cs16", "channel": 0, "antenna": "RX"}`

Constraints:

- Only one receiver may use `{"kind": "stdin", ...}`.

Supported `format` values: `u8`, `s8`, `u16`, `s16`, `cs16`, `f32`, `cf32`, `f64`.

#### SoapySDR driver options

Extra keys supported for `{"kind":"soapysdr", ...}`:

| Key | Type | Notes |
|---|---:|---|
| `agc` | bool | When set, forces SoapySDR RX gain mode on/off (device must support it) |
| `gain` | number | Sets overall RX gain in dB |
| `gains` | object | Per-gain-element dB values (keys must match `Device::list_gains`) |
| `settings` | object | Raw SoapySDR device settings (written via `write_setting`) |
| `stream_args` | object | Raw SoapySDR stream arguments (passed to `Device::rx_stream_args`) |
| `rx_buffer_samples` | int | Internal SoapySDR read buffer size in samples (per `readStream` call). Larger values reduce call overhead and can reduce overflows at high sample rates. |

### `receivers[].input.defaults`

| Key | Type | Notes |
|---|---:|---|
| `frequency` | int | `-1` means "center" |
| `modulation` | string | `USB`, `LSB`, `AM`, `SAM`, `FM`, `FMC`, `WBFM` |
| `ssb_lowcut_hz` | int | Optional. Default `300`. Only used when `modulation` is `USB`/`LSB`. |
| `ssb_highcut_hz` | int | Optional. Default `3000`. Only used when `modulation` is `USB`/`LSB`. Must be `> ssb_lowcut_hz`. |

The backend clamps the derived default `(l,r)` audio window to `audio_max_fft_size` so `/audio` always starts.

Default audio window shapes (derived from `defaults.modulation`):
- `USB`: `+ssb_lowcut_hz..+ssb_highcut_hz` relative to the tuned carrier (defaults: `+300..+3000 Hz`)
- `LSB`: `-ssb_highcut_hz..-ssb_lowcut_hz` relative to the tuned carrier (defaults: `-3000..-300 Hz`)
- `AM` / `SAM` / `FM`: `±5 kHz`
- `FMC`: `±5 kHz` (frontend applies an extra ~300 Hz high-pass to reduce CTCSS)
- `WBFM`: `±96 kHz` (default only; usable width is limited by `audio_sps`)

## `bands.json`

This file is optional. When present, the UI uses it for band overlays and the band jump menu.
Location: `config/overlays/bands.json` next to `config/config.json`.

Supported shapes:

- Array: `[{ "name": "...", "startHz": 0, "endHz": 0, "color": "rgba(...)" }]`
- Object wrapper: `{ "bands": [ ... ] }`

Only `name`, `startHz`, and `endHz` are required. `color` is optional.

## `markers.json`

This file is optional. When present, the UI shows markers in the waterfall scale.
Location: `config/overlays/markers.json` next to `config/config.json`.

The supported schema is documented in the UI code (it accepts multiple key aliases), but the most direct form is:

```json
{ "markers": [{ "frequency": 7074000, "name": "FT8", "mode": "USB" }] }
```
