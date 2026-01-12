# Configuration

NovaSDR reads two JSON files by default:

- `config/config.json` (global server/WebSDR settings)
- `config/receivers.json` (receiver DSP + input settings)

If you prefer an interactive wizard, run:

- `novasdr-server setup -c config/config.json -r config/receivers.json`
- `novasdr-server configure -c config/config.json -r config/receivers.json` (edit an existing config with the same wizard)

NovaSDR also optionally reads additional JSON files from `config/overlays/` next to `config/config.json`.
If the files are missing, NovaSDR creates defaults on startup (empty markers; a basic band plan).

- `config/overlays/markers.json` (UI markers; hot-reloaded about once per minute)
- `config/overlays/bands.json` (band plan overlays and band jump list; hot-reloaded about once per minute)

You can edit/reset these overlays from the setup wizard (`setup` / `configure`), or by editing the files directly.

The Rust backend supports:
- `receivers[].input.waterfall_compression = "zstd"`
- `receivers[].input.audio_compression = "adpcm"`
- `receivers[].input.accelerator = "none"` (or `clfft` with the `clfft` feature; or `vkfft` with the `vkfft` feature)

## Online listing registration

NovaSDR can periodically publish basic server information to an SDR list service.

Configuration:

- `websdr.register_online = true`
- `websdr.register_url = "https://sdr-list.xyz/api/update_websdr"` (default)

The registration payload includes server name, antenna, grid locator, hostname, port, user count, bandwidth and center frequency.

## Minimal working config

`config/config.json`:

```json
{
  "server": { "host": "[::]", "port": 9002, "html_root": "frontend/dist/", "otherusers": 1, "threads": 0 },
  "websdr": { "name": "NovaSDR", "operator": "operator", "email": "operator@example.com", "grid_locator": "-", "chat_enabled": true },
  "limits": { "audio": 1000, "waterfall": 1000, "events": 1000, "ws_per_ip": 50 },
  "updates": { "check_on_startup": true, "github_repo": "Steven9101/NovaSDR" },
  "active_receiver_id": "rx0"
}
```

`config/receivers.json`:

```json
{
  "receivers": [
    {
      "id": "rx0",
      "name": "Main",
      "input": {
        "sps": 2048000,
        "frequency": 100900000,
        "signal": "iq",
        "fft_size": 131072,
        "audio_sps": 12000,
        "waterfall_size": 1024,
        "waterfall_compression": "zstd",
        "audio_compression": "adpcm",
        "accelerator": "none",
        "smeter_offset": 0,
        "driver": { "kind": "stdin", "format": "u8" },
        "defaults": { "frequency": -1, "modulation": "USB", "squelch_enabled": false }
      }
    }
  ]
}
```

> [!NOTE]
> `server.html_root` must point to the built frontend output directory (typically `frontend/dist/`).
>
> `server.threads = 0` enables an auto-selected Tokio worker thread count based on available CPU cores.

## Important derived values

Some runtime values are derived from the configuration:

- `fft_result_size`:
  - `signal = "iq"`: `fft_size`
  - `signal = "real"`: `fft_size / 2`
- `basefreq`:
  - `signal = "iq"`: `frequency - (sps / 2)`
  - `signal = "real"`: `frequency`
- `audio_max_fft_size`:
  - `ceil(audio_sps * fft_size / sps / 4) * 4`

`audio_sps` must be `<= 48000`.

The server clamps the default audio window (`receivers[].input.defaults`) to fit into `audio_max_fft_size` so audio always starts.

## Optional: USB/LSB default passband

By default, NovaSDR uses:

- `USB`: `+300..+3000 Hz`
- `LSB`: `-3000..-300 Hz`

To override that per receiver, add `ssb_lowcut_hz` / `ssb_highcut_hz` under `receivers[].input.defaults`:

```json
{
  "defaults": {
    "frequency": -1,
    "modulation": "USB",
    "ssb_lowcut_hz": 100,
    "ssb_highcut_hz": 2800
  }
}
```

## Input sample formats

`receivers[].input.driver.format` defines how input bytes are converted to `f32`:

- `u8`, `s8`
- `u16`, `s16`
- `f32`, `f64`
- `cs16`, `cf32` (interleaved IQ, re/im pairs)

Match this to the selected input source (stdin tool output or SoapySDR device format).

## Input sources

NovaSDR supports multiple sample sources. Select the source via `receivers[].input.driver.kind`.

NovaSDR can be configured with **multiple receivers** in `config/receivers.json`. The server runs one DSP pipeline per receiver and the frontend can switch between them at runtime.

Constraints:

- `driver.kind = "soapysdr"` supports multiple receivers (feature-gated).
- Only **one** receiver may use `driver.kind = "stdin"` (stdin is a single stream).

### `soapysdr` (feature-gated)

SoapySDR input is available behind the `soapysdr` feature flag.

Example `config/receivers.json` snippet:

```json
{
  "id": "rx0",
  "input": {
    "sps": 2048000,
    "frequency": 100900000,
    "signal": "iq",
    "driver": {
      "kind": "soapysdr",
      "device": "driver=rtlsdr",
      "channel": 0,
      "format": "cs16",
      "agc": false,
      "gain": 35.0,
      "gains": { "LNA": 30.0 },
      "settings": { "biastee": "true" },
      "stream_args": { "buffers": "16", "bufflen": "131072" },
      "rx_buffer_samples": 131072
    }
  }
}
```

### `stdin`

This is the default mode and expects raw sample bytes on standard input.

Example:

- `rtl_sdr -g 48 -f 100900000 -s 2048000 - | ./target/release/novasdr-server -c config/config.json -r config/receivers.json`

<details>
<summary><strong>Choosing a sensible FFT size</strong></summary>

The FFT size determines frequency resolution and processing load. At high sample rates, the FFT must complete before the next frame arrives.

**Processing time budget:**

```
time_per_frame = (fft_size / 2) / sample_rate
```

**Guidelines:**

- **Target frame rate:** 20-60 fps is a good practical range
- **Frequency resolution:** `sample_rate / fft_size` Hz per bin
- **Processing headroom:** Larger FFT needs more time; ensure frame budget is adequate

**Recommended FFT sizes:**

| Sample Rate | Recommended `fft_size` | Typical frame rate | Frequency resolution |
|-------------|------------------------|------------|---------------------|
| 0-3 MHz     | 131,072               | 20-45 fps  | 15-23 Hz/bin        |
| 3-10 MHz    | 131,072 - 262,144     | 20-75 fps  | 11-76 Hz/bin        |
| 10-30 MHz   | 524,288 - 1,048,576   | 20-60 fps  | 10-57 Hz/bin        |
| 30-100 MHz  | 524,288 - 1,048,576   | 25-95 fps  | 29-190 Hz/bin       |

**Example calculations (20 MSPS):**

- With `fft_size = 524288`:
  - Frame rate ~ `20,000,000 / (524,288 / 2)` ~ 76 fps
  - Time budget ~ 13 ms per frame
- With `fft_size = 1048576`:
  - Frame rate ~ `20,000,000 / (1,048,576 / 2)` ~ 38 fps
  - Time budget ~ 26 ms per frame

In practice, for 10-30 MSPS, `fft_size` in the **500k-1M** range is a common sweet spot. For example, an Intel HD 530-class machine can run this range with headroom (observed ~60% utilization in typical configurations).

**Performance tips:**

1. Start with a larger FFT at high sample rates (10-30 MSPS: 500k-1M), then adjust based on CPU/GPU load.
2. Use `accelerator = "clfft"` (GPU) for high sample rates (requires `--features clfft`).
3. Monitor logs for `waterfall frame skip` to see how much work is being throttled under load.
4. See [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for tuning guidance.

If you are unsure, start with `131072` at `sps` up to 3 MSPS, then tune based on CPU usage and logs.

</details>
