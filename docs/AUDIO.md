# Audio Pipeline

Audio is derived from slices of the FFT spectrum and streamed to browsers.

## Overview

```mermaid
graph LR
  A[Spectrum slice Complex32] --> B[Bin mapping to baseband]
  B --> C[IFFT to time domain]
  C --> D[Overlap add 50 percent]
  D --> E[DC removal]
  E --> F[AGC]
  F --> G[Quantize i16]
  G --> H[FLAC encode 16 bit]
  H --> I[CBOR packet]
  I --> J[audio websocket frames]
```

## Modes

The server accepts demodulation changes from the frontend:

```json
{ "cmd": "demodulation", "demodulation": "USB" }
```

Supported mode strings:
- `USB`, `LSB`, `AM`, `FM`, `FMC`, `SAM`

`FMC` is an alias of `FM` on the backend (the extra CTCSS reduction is a frontend audio filter).

## Squelch (auto, frequency-domain)

The WebSDR squelch is implemented server-side and operates on the current audio window in the frequency domain.
It does not use a user-set signal-level threshold.

Frontend command:

```json
{ "cmd": "squelch", "enabled": true }
```

Algorithm (per audio frame):
- Compute per-bin power over the audio FFT slice:
  - `p_i = |X_i|^2`
- Compute relative variance:
  - `rv = var(p) / mean(p)^2`
- Compute a bandwidth-independent score (where `N` is the number of bins):
  - `scaled = (rv - 1) * sqrt(N)`

Decision logic (fixed constants):
- Open immediately if `scaled >= 18`.
- Open if `scaled >= 5` for 3 consecutive frames.
- When open, close only after `scaled < 2` for 10 consecutive frames (hysteresis).

When squelch is enabled and closed, the server does not emit audio packets.
## Output format (frontend contract)

The frontend uses a persistent decoder instance and expects to receive chunks of a FLAC stream.

Each WebSocket binary frame is CBOR encoding of:

```text
{
  frame_num: u64,
  l: i32,
  m: f64,
  r: i32,
  pwr: f32,
  data: bytes (FLAC stream bytes)
}
```

See: `crates/novasdr-core/src/protocol.rs` (`AudioPacket`)

## 16-bit FLAC

For bandwidth and compatibility with the existing frontend decoder, the backend produces 16‑bit mono FLAC.

- DSP produces `f32` audio samples.
- Samples are quantized to signed 16-bit centered PCM.
- The FLAC encoder is configured for `bits_per_sample = 16`.

Implementation:
- Quantizer: `crates/novasdr-core/src/dsp/demod.rs`
- Pipeline: `crates/novasdr-server/src/ws/audio.rs`

## Window sizing and `audio_max_fft_size`

The server only processes audio windows up to `audio_max_fft_size` bins.

This size is derived from:

```text
audio_max_fft_size = ceil(audio_sps * fft_size / sps / 4) * 4
```

The runtime clamps default `(l,r)` to this maximum to guarantee audio starts even for wideband defaults.

Note: `audio_max_fft_size` is not required to be a power-of-two (FFTW supports arbitrary sizes). The Rust implementation uses `rustfft` for the inverse transform, which also supports non-power-of-two sizes.
