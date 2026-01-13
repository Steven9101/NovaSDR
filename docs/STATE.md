# State and Concurrency

## Threads / tasks

```mermaid
sequenceDiagram
  participant DSP as DSP thread
  participant WS as Tokio WS tasks
  participant State as Shared State

  WS->>State: register clients (audio/waterfall/events/chat)
  DSP->>State: read client params (audio is lock-free)
  DSP->>WS: push encoded packets to per-client channels
  WS->>Client: websocket send
```

## Shared structures

Implementation: `crates/novasdr-server/src/state.rs`

- `DashMap` for client registries (fast concurrent access)
- Audio params stored in atomics (DSP reads lock-free)
- Per-client `Mutex` for DSP pipelines (and waterfall params)
- Atomic counters for bitrate accounting

## Marker updates

`config/overlays/markers.json` is polled periodically and embedded into the initial settings JSON.
