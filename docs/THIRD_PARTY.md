# Third-party software and attribution

This repository includes third-party software and derived work. This document exists to:

- credit upstream authors and maintainers
- identify bundled artifacts (including WebAssembly)
- make license compliance straightforward for downstream redistributors

If anything here is missing or incorrect, please open a pull request.

## Upstream projects

NovaSDR is a continuation of PhantomSDR-plus (a fork maintained by `magicint1337`) and is based on earlier WebSDR work. A major upstream reference project is maintained by **@rhgndf** ([GitHub profile](https://github.com/rhgndf)).

## Bundled generated artifacts

The repository includes generated artifacts that are distributed as part of NovaSDR:

### `novasdrdsp` WebAssembly modules

Files under `frontend/src/modules/`:

- `novasdrdsp_bg.wasm`
- `novasdrdsp_bg.js`
- `novasdrdsp.js`
- `novasdrdsp.d.ts`
- `novasdrdsp_bg.wasm.d.ts`

These modules are used by the frontend for:

- audio decoding (`frontend/src/components/audio/useAudioClient.ts`)
- waterfall stream decoding (`frontend/src/components/waterfall/WaterfallView.tsx`)

Attribution: these modules are derived from upstream work in `PhantomSDR/decoders` (Apache-2.0): https://github.com/PhantomSDR/decoders

Source repository for the NovaSDR build: https://github.com/Steven9101/novasdr-wasm

### FT8 decoder (Emscripten output)

Files under `frontend/src/decoders/ft8/emscripten/`:

- `decode_ft8.wasm`
- `decode_ft8.js`
- `decode_ft8.js.d.ts`

These artifacts are executed in a Web Worker (`frontend/src/decoders/ft8/ft8Worker.ts`).

Attribution: these files are derived from **`ft8js`** (WASM-compiled `ft8_lib`) by **@e04**:

- Upstream: [e04/ft8js](https://github.com/e04/ft8js)

The upstream `ft8js` project vendors `ft8_lib` as a submodule and builds browser-ready WebAssembly outputs. See the upstream repository for the authoritative license terms for `ft8js` and its `ft8_lib` submodule.

## Direct dependencies (non-exhaustive)

The lists below are the **direct** dependencies declared in this repository. They are not a complete transitive inventory.

### Rust crates (direct)

- `crates/novasdr-core/Cargo.toml`
- `crates/novasdr-server/Cargo.toml`

### Frontend npm packages (direct)

- `frontend/package.json`

## Generating full license reports (recommended for releases)

For redistributors who need a complete list of transitive licenses, generate and archive a report as part of your release process.

- Rust (transitive): use a license reporting tool (for example, `cargo-about` or `cargo-deny`) and store the output alongside release artifacts.
- Frontend (transitive): generate an npm dependency license report from `frontend/package-lock.json`.

