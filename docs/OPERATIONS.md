# Operations

## Running behind a reverse proxy

NovaSDR serves static UI and WebSockets from the same origin. If you reverse-proxy:
- forward `Upgrade` and `Connection` headers
- increase timeouts for long-lived sockets

## Resource sizing

CPU usage is dominated by:
- FFT execution (`input.fft_size`)
- number of connected clients (per-client demod + compression)

Memory pressure is driven by:
- FFT buffers
- slow WebSocket clients (bounded queues; audio/waterfall frames may be dropped)

## Observability

NovaSDR uses `tracing` and writes logs to stderr by default.

Configuration:

- `--debug` enables more verbose logs for the NovaSDR crates.
- `RUST_LOG` overrides filtering completely (example: `RUST_LOG=info,novasdr_server=debug`).
- By default, logs are also written to rotating files under `./logs/`. Disable with `--no-file-log`.

Operational signals:

- Slow clients are protected by bounded per-client queues; when the queue is full, audio/waterfall frames are dropped for that client rather than buffering unbounded memory.
- If you expect many clients, tune `[limits]` and consider increasing queue sizes in `crates/novasdr-server/src/state.rs`.

<details>
<summary><strong>Operational checklist</strong></summary>

- Verify the UI assets are being served from `server.html_root`.
- Verify `/audio`, `/waterfall`, `/events` WebSockets connect (browser devtools).
- Tune `[limits]` to match the deployment expectations.
- If running behind a proxy, confirm WebSocket upgrade headers are forwarded.
- Enable `--debug` temporarily when diagnosing client disconnects.

</details>
