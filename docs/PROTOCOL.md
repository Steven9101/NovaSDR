# Client protocol

NovaSDR serves:

- HTTP static UI from `server.html_root`
- `GET /server-info.json` (JSON)
- `GET /receivers.json` (JSON; list of configured receivers)
- WebSockets:
  - `/waterfall` (text JSON settings, then binary zstd+CBOR packets)
  - `/audio` (text JSON settings, then binary framed packets)
  - `/events` (text JSON, periodic updates)
  - `/chat` (text JSON)

## Initial settings message (text JSON)

On `/audio` and `/waterfall`, the first WebSocket message is a JSON object containing:

- `sps`, `fft_size`, `fft_result_size`, `basefreq`, `total_bandwidth`
- `defaults` (default tuning window + mode)
  - `defaults.squelch_enabled` (optional; if present, clients may enable squelch automatically)
- `waterfall_compression` (`"zstd"`)
- `audio_compression` (`"adpcm"`)
- `overlap`, `fft_overlap` (both `fft_size/2` for the 50 percent overlap model)
- `markers` (stringified JSON; optional file `config/overlays/markers.json`)
- `bands` (stringified JSON; optional file `config/overlays/bands.json`)

This settings message may be sent again later (for example after a receiver switch via `cmd = "receiver"`). The frontend expects a settings message before any subsequent binary stream restart.

## WebSocket commands (JSON)

Clients send JSON objects with `cmd`:
- `receiver` (`receiver_id`)
- `window` (`l`, `r`, optional `m`, optional `level`)
- `demodulation` (`demodulation`)
- `mute` (`mute`)
- `squelch` (`enabled`)
- `agc` (`speed`, optional `attack`, optional `release`)
- `chat` (`username`, `message`, optional `user_id`, optional `reply_to_id`, optional `reply_to_username`)

Notes:
- For `/audio`, `m` is the tuned center bin and may be outside the selected window (for example SSB low-cut windows like USB `+100..+2800 Hz` or LSB `-2800..-100 Hz` relative to `m`).

## `/waterfall` binary frames

Binary WebSocket frames are Zstd-stream-compressed CBOR packets.

CBOR schema (map):

```text
{
  frame_num: u64,
  l: i32,
  r: i32,
  data: bytes (i8 intensity values)
}
```

The frontend decodes: Zstd stream → CBOR → `Int8Array`.

## `/audio` binary frames

Binary WebSocket frames are a custom binary envelope (little-endian) followed by codec payload bytes.

Header (36 bytes):

```text
0..4    magic = "NSDA"
4       version = u8 (1)
5       codec = u8 (1=IMA ADPCM)
6..8    reserved = u16 (0)
8..16   frame_num = u64
16..20  l = i32 (window start index)
20..28  m = f64 (tuned center bin)
28..32  r = i32 (window end index)
32..36  pwr = f32
36..    payload bytes
```

Notes:
- For the current audio stream implementation, `l`/`r` in the audio header refer to indices within the spectrum slice used for demodulation, not absolute bins in the full FFT result. Today the server sends `l=0` and `r=slice_len`.
- `pwr` is the average power across the same slice that produced the audio.

Payload:
- codec `1` (IMA ADPCM, mono): a single self-contained block:
  - `predictor: i16`, `index: u8`, `reserved: u8`, `sample_count: u16`, then 4-bit ADPCM codes packed low-nibble first.
