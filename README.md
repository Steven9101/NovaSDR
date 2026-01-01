# NovaSDR

<p>
  <img alt="License" src="https://img.shields.io/badge/license-GPL--3.0--only-blue">
  <img alt="Backend" src="https://img.shields.io/badge/backend-Rust-orange">
  <img alt="Audio" src="https://img.shields.io/badge/audio-FLAC-informational">
  <img alt="Waterfall" src="https://img.shields.io/badge/waterfall-Zstd-informational">
  <img alt="Transport" src="https://img.shields.io/badge/transport-WebSocket-informational">
</p>

NovaSDR is a WebSDR server that streams **waterfall/spectrum** and **demodulated audio** to a browser UI.

- **Backend**: Rust (SoapySDR + OpenCL clFFT in the recommended build; CPU-only builds are supported)
- **Codecs**: waterfall `zstd`, audio `flac`
- **Transport**: WebSockets for real-time streams + HTTP for static UI

## What this repository contains

- `crates/novasdr-server/`: server, WebSocket endpoints, DSP runner
- `crates/novasdr-core/`: configuration, DSP primitives, codecs, protocol types
- `frontend/`: React UI (shadcn/ui), built as static files and served by the server
- `docs/`: maintained documentation (start at `docs/index.md`)

## Installation

### One-line installer (Linux/macOS)

> Installer URL:

```sh
  curl -fsSL https://novasdr.com/install.sh | sh
```

The installer supports:

- Building NovaSDR from source
- Building and installing SoapySDR from source (always; recommended for direct hardware access)
- Installing OpenCL headers/runtime (optional)
- Installing Vulkan/glslang/SPIRV-Tools (optional; needed for `--features vkfft` builds)
- Building a SoapySDR device module from source (RTL-SDR, HackRF, Airspy, etc. where supported)

See `docs/INSTALL.md` for details.

### Linux packages (manual)

NovaSDR runs either with:

- **SoapySDR** (recommended; direct hardware access), or
- **stdin** (pipe raw samples from an external capture tool).

The sections below are copy/paste one-liners for a full source build (Rust + frontend + SoapySDR + OpenCL + clFFT).

<details>
<summary><strong>Debian/Ubuntu (apt)</strong></summary>

```sh
sudo apt-get update && sudo apt-get install -y --no-install-recommends \
  ca-certificates curl git tar \
  build-essential cmake pkg-config \
  clang libclang-dev \
  swig python3 python3-dev python3-numpy \
  nodejs npm \
  ocl-icd-opencl-dev ocl-icd-libopencl1 \
  libclfft-dev \
  libusb-1.0-0-dev
```

</details>

<details>
<summary><strong>Fedora (dnf)</strong></summary>

```sh
sudo dnf install -y \
  ca-certificates curl git tar \
  gcc gcc-c++ make cmake pkgconf-pkg-config \
  clang llvm-devel libclang-devel \
  swig python3 python3-devel python3-numpy \
  nodejs npm \
  ocl-icd ocl-icd-devel \
  libusb1-devel
```

</details>

<details>
<summary><strong>Arch (pacman)</strong></summary>

```sh
sudo pacman -Sy --noconfirm --needed \
  ca-certificates curl git tar \
  base-devel cmake pkgconf \
  clang llvm libclang \
  swig python python-numpy \
  nodejs npm \
  ocl-icd opencl-headers \
  libusb
```

</details>

<details>
<summary><strong>openSUSE (zypper)</strong></summary>

```sh
sudo zypper --non-interactive refresh && sudo zypper --non-interactive install -y \
  ca-certificates curl git tar \
  gcc-c++ make cmake pkg-config \
  clang llvm llvm-devel libclang-devel \
  swig python3 python3-devel python3-numpy \
  nodejs npm \
  OpenCL-Headers ocl-icd-devel \
  libusb-1_0-devel
```

</details>

<details>
<summary><strong>macOS (Homebrew)</strong></summary>

```sh
brew update && brew install \
  git cmake pkg-config \
  llvm \
  swig python \
  node \
  libusb
```

</details>

> [!NOTE]
> If you see a `bindgen` error like `Unable to find libclang`, set `LIBCLANG_PATH` (for example on macOS: `export LIBCLANG_PATH="$(brew --prefix llvm)/lib"`), then rebuild.

## Build (backend + UI)

```bash
cargo build -p novasdr-server --release --features "soapysdr,clfft"
cd frontend && npm ci && npm run build && cd ..
```

If you want stdin-only mode, omit `soapysdr`. If you want a CPU-only build, omit `clfft`.

## Configure

Recommended: run the interactive wizard:

```bash
./target/release/novasdr-server setup -c config/config.json -r config/receivers.json
```

To edit an existing configuration with the same wizard:

```bash
./target/release/novasdr-server configure -c config/config.json -r config/receivers.json
```

Or edit `config/config.json` and `config/receivers.json` manually:

- `config/config.json`: server/websdr/limits + `active_receiver_id`
- `config/receivers.json`: receiver + input settings (sample rate, FFT size, sample format, signal type)

## Run

```bash
./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

Open: `http://localhost:9002`

## SoapySDR mode (feature-gated)

With `--features soapysdr`, NovaSDR can open SDR devices directly from `config/receivers.json` (`receivers[].input.driver.kind = "soapysdr"`), or you can run `novasdr-server setup` to scan devices and generate receivers. In the recommended build we enable both `soapysdr` and `clfft`.

SoapySDR itself is a system library. For maximum compatibility, build SoapySDR from source:

```bash
git clone https://github.com/pothosware/SoapySDR.git
cd SoapySDR
mkdir build && cd build
cmake ..
make -j"$(nproc)"
sudo make install
sudo ldconfig # Debian/Ubuntu
SoapySDRUtil --info
```

## Device examples

### SoapySDR (recommended)

The simplest workflow is:

1. Build with `--features "soapysdr,clfft"`
2. Run `novasdr-server setup` and select a device (first launch will prompt automatically when config files are missing)
3. Run `novasdr-server`

If you prefer to write `config/receivers.json` manually, an RTL-SDR example looks like:

```json
{
  "kind": "soapysdr",
  "device": "driver=rtlsdr",
  "channel": 0,
  "format": "cs16",
  "agc": false,
  "gain": 35.0,
  "gains": { "LNA": 30.0 },
  "settings": { "biastee": "true" }
}
```

### stdin (pipe)

Make sure `input.driver.format` and `input.signal` match what the capture tool outputs.

### RTL-SDR (IQ, `u8`)

```bash
rtl_sdr -g 48 -f 100900000 -s 2048000 - | ./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

### HackRF (IQ, `s8`)

```bash
hackrf_transfer -r - -f 100900000 -s 8000000 | ./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

### Airspy HF+ (IQ, `s16`)

```bash
airspy_rx -r - -f 648000 -s 912000 | ./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

### RX888 MK2 (real, `s16`)

```bash
rx888_stream -s 6000000 | ./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

## Documentation

Start here: `docs/index.md`

Key docs:

- `docs/BUILDING.md`
- `docs/CONFIG.md` and `docs/CONFIG_REFERENCE.md`
- `docs/PROTOCOL.md`
- `docs/ARCHITECTURE.md`
- `docs/DSP.md`, `docs/AUDIO.md`, `docs/WATERFALL.md`
- `docs/OPERATIONS.md` and `docs/TROUBLESHOOTING.md`
- `docs/LICENSING.md` and `docs/THIRD_PARTY.md`

The GitHub wiki is published from `docs/` (see `tools/publish_wiki.sh`).
On Windows, use `tools/publish_wiki.ps1`.

## License

NovaSDR is licensed under GPLv3.

- `LICENSE`
- `NOTICE`

## Attribution

NovaSDR is a continuation of PhantomSDR-plus (maintained by `magicint1337`) and includes contributions from NovaSDR contributors.

Copyright (c) 2025-2026, magicint1337.  
Copyright (c) 2025-2026, NovaSDR contributors.

See `NOTICE` and `docs/LICENSING.md` for details and upstream attribution.
