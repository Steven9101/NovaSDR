# Troubleshooting

## Fast checks

> [!TIP]
> Use the probe tool to confirm the server is actually streaming:
>
> ```bash
> cargo build -p ws_probe
> ./target/debug/ws_probe ws://127.0.0.1:9002/audio
> ./target/debug/ws_probe ws://127.0.0.1:9002/waterfall
> ```

## No audio

Checklist:

- Confirm the `/audio` WebSocket receives the initial JSON settings.
- Confirm `input.defaults.modulation` and the resulting default `(l,r)` window are within `audio_max_fft_size`.
- If using stdin input, confirm the sample format matches `receivers[].input.driver.format`.
- Confirm the browser console has no decoder errors.

## Waterfall works, audio silent

Common causes:

- Default audio window too wide for the current `sps`/`fft_size` (the server clamps defaults, but custom ranges may be rejected).
- Sample format mismatch (e.g., `u8` vs `s16`).

<details>
<summary><strong>Symptoms and likely causes</strong></summary>

| Symptom | Likely cause | Where to look |
|---|---|---|
| `/waterfall` works, `/audio` connects but no sound | wrong modulation/window, muted, decoder issue | browser console, [Audio](AUDIO.md), [Protocol](PROTOCOL.md) |
| WebSocket closes immediately | proxy missing upgrade headers | [Operations](OPERATIONS.md) |
| S-meter updates but audio does not | audio commands not applied or rejected | browser console, server logs with `--debug` |
| High latency / stutter | slow client, CPU bound | [Operations](OPERATIONS.md) |

</details>

## High CPU

Reduce:

- `input.fft_size` (first)
- then `input.sps`

> [!NOTE]
> NovaSDR prioritizes real-time behavior: slow clients drop frames instead of buffering unbounded memory.

## Input underruns at high sample rates

At very high sample rates, the DSP loop has less time per FFT frame. If the server can’t keep up, you’ll see dropped frames and stuttering.

Each FFT frame must be processed within a time window determined by:

```
time_per_frame = (fft_size / 2) / sample_rate
```

Examples (idealized):

| Sample Rate | FFT Size | Frame Rate | Processing Budget |
|-------------|----------|------------|-------------------|
| 2 MHz       | 131,072  | ~30 fps    | ~33 ms/frame     |
| 20 MHz      | 131,072  | ~305 fps   | ~3.3 ms/frame    |
| 20 MHz      | 1,048,576| ~38 fps    | ~26 ms/frame     |
| 60 MHz      | 131,072  | ~915 fps   | ~1.1 ms/frame    |
| 60 MHz      | 1,048,576| ~114 fps   | ~8.7 ms/frame    |

Practical levers:

- Lower `input.sps` and/or `input.fft_size` to reduce total work.
- If you are GPU-bound or want more headroom, enable OpenCL acceleration with `accelerator = "clfft"` and build with `--features clfft`.
