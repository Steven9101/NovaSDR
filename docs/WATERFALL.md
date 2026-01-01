# Waterfall Pipeline

## Overview

Waterfall data is generated in the FFT stage and sent as:

1) CBOR packet (metadata + `i8` intensity slice)  
2) Zstandard stream compression (`flush` per message)  
3) WebSocket binary frame

## Rendering (frontend)

The React frontend (`frontend/src/components/waterfall/WaterfallView.tsx`) renders the waterfall:

- Newest row is drawn at the bottom and the image scrolls upward.
- The frequency scale (band plan + ticks) and passband tuner bar are rendered below the waterfall.

## Packet format

The payload inside the Zstd stream is CBOR encoding of:

```text
{
  frame_num: u64,
  l: i32,
  r: i32,
  data: bytes (interpreted by frontend as Int8Array)
}
```

See: `crates/novasdr-core/src/protocol.rs` (`WaterfallPacket`)

## Client window selection

Clients send:

```json
{ "cmd": "window", "l": 123, "r": 456 }
```

The backend chooses an appropriate downsample level so the number of samples sent is near `input.waterfall_size`.

## Level selection

Implementation: `crates/novasdr-server/src/ws/waterfall.rs`

The server re-maps the requested `(l,r)` window across downsample levels until the window width is closest to `input.waterfall_size`.

