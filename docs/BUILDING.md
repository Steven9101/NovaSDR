# Building (Rust backend)

## Prereqs

- Rust toolchain (stable)
- Node.js + npm (frontend build)

> [!TIP]
> For best performance, use a native toolchain on Linux and build in `--release`.

## Repository checkout

```bash
git clone --recurse-submodules https://github.com/Steven9101/NovaSDR.git
cd NovaSDR
```

## Backend

Recommended (SoapySDR + OpenCL clFFT):

```bash
cargo build -p novasdr-server --release --features "soapysdr,clfft"
```

### Install Rust (rustup)

If you don't have `cargo` yet:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After install, you may need to restart your shell or source the env file:

```bash
. "$HOME/.cargo/env"
```

<details>
<summary><strong>clFFT (OpenCL) acceleration</strong></summary>

NovaSDR uses `clFFT` for the forward FFT when `input.accelerator = "clfft"`.

Build:

```bash
cargo build -p novasdr-server --release --features "soapysdr,clfft"
```

Notes:

- Requires an OpenCL runtime/driver for your GPU.
- Requires the `clFFT` library to be installed and discoverable by the linker/loader.
- Both IQ and real-input forward FFT are GPU-backed when `clfft` is enabled.

Install `clFFT` from your distro packages.

Example (Debian/Ubuntu):

```bash
sudo apt-get install -y --no-install-recommends libclfft-dev
```

Select platform/device (optional):

- `NOVASDR_OPENCL_PLATFORM=0`
- `NOVASDR_OPENCL_DEVICE=0`

</details>

<details>
<summary><strong>VkFFT (Vulkan) acceleration (Raspberry Pi 4 / Vulkan GPUs)</strong></summary>

NovaSDR can optionally use `VkFFT` for the forward FFT when `input.accelerator = "vkfft"` and the backend is built with the `vkfft` feature.

Build:

```bash
cargo build -p novasdr-server --release --features "soapysdr,vkfft"
```

Notes:

- Linux only.
- Requires a Vulkan-capable driver/stack.
- Uses a small C++ wrapper built at compile time (requires a C++ toolchain).
- Real-input forward FFT currently falls back to CPU when `vkfft` is selected.
- Waterfall quantization/downsampling can also run on the GPU, but it is scheduled off the DSP critical path (it runs on the dedicated waterfall worker thread when available).

Example packages (Debian / Raspberry Pi OS):

```bash
sudo apt-get install -y --no-install-recommends \
  libvkfft-dev \
  libvulkan-dev \
  glslang-dev \
  spirv-tools \
  pkg-config \
  g++
```

Raspberry Pi 4 notes:

- The Pi 4 GPU is limited; expect lower sustainable sample rates than a desktop GPU.
- A known-good baseline is `sps = 4000000` with SDRplay (SoapySDRPlay) and `accelerator = "vkfft"`.
- Verify Vulkan is working:

```bash
vulkaninfo | grep -E 'deviceName|driverInfo' || true
```

Select Vulkan device (optional):

- `NOVASDR_VULKAN_DEVICE=0` (or `1`, etc.)

</details>

<details>
<summary><strong>SoapySDR input (feature-gated)</strong></summary>

NovaSDR can optionally capture samples directly from SoapySDR devices when built with the `soapysdr` feature.

Build:

```bash
cargo build -p novasdr-server --release --features "soapysdr,clfft"
```

Notes:

- Requires the SoapySDR library installed on the system.
- Requires a SoapySDR device module for your SDR (e.g. `SoapyRTLSDR` for RTL-SDR).
- Requires `pkg-config` (and a `SoapySDR.pc`) so the build can find headers/libs.
- Requires `libclang` (bindgen runs at build time). On Ubuntu/Debian this is typically `clang` + `libclang-dev`.
  - In CI/cross builds, the bindgen step runs on the build machine. Ensure the build environment has a working `libclang.so` and, if necessary, set `LIBCLANG_PATH` to the directory containing it (for example via `llvm-config --libdir`).
- This feature is intended primarily for Linux; Windows support depends on your SoapySDR install/toolchain.

### SoapySDR (from source, Debian/Ubuntu)

This is useful if your distro does not ship SoapySDR packages or you need a newer version.

```bash
sudo apt-get install cmake g++ libpython3-dev python3-numpy swig
git clone https://github.com/pothosware/SoapySDR.git
cd SoapySDR
mkdir build
cd build
cmake ..
make -j"$(nproc)"
sudo make install
sudo ldconfig
SoapySDRUtil --info
```

### SoapyRTLSDR (from source, Debian/Ubuntu)

```bash
sudo apt-get install rtl-sdr librtlsdr-dev
git clone https://github.com/pothosware/SoapyRTLSDR.git
cd SoapyRTLSDR
mkdir build
cd build
cmake ..
make -j"$(nproc)"
sudo make install
SoapySDRUtil --probe="driver=rtlsdr"
```

### RTL-SDR v4 (Debian/Ubuntu driver rebuild)

This replaces system RTL-SDR drivers. Review carefully before running.

```bash
sudo apt purge ^librtlsdr
sudo rm -rvf /usr/lib/librtlsdr* /usr/include/rtl-sdr* /usr/local/lib/librtlsdr* /usr/local/include/rtl-sdr* /usr/local/include/rtl_* /usr/local/bin/rtl_*
sudo apt-get install libusb-1.0-0-dev git cmake pkg-config build-essential
git clone https://github.com/osmocom/rtl-sdr
cd rtl-sdr
mkdir build
cd build
cmake ../ -DINSTALL_UDEV_RULES=ON
make -j"$(nproc)"
sudo make install
sudo cp ../rtl-sdr.rules /etc/udev/rules.d/
sudo ldconfig
echo 'blacklist dvb_usb_rtl28xxu' | sudo tee --append /etc/modprobe.d/blacklist-dvb_usb_rtl28xxu.conf
```

</details>

## Frontend

```bash
cd frontend
npm ci
npm run build
cd ..
```

If `npm ci` fails due to a missing lockfile, run `npm install` once to generate `frontend/package-lock.json`, then use `npm ci` again.

## Run

NovaSDR reads raw samples from stdin by default. The recommended build enables `--features "soapysdr,clfft"`: SoapySDR provides device input, and `clfft` enables optional OpenCL acceleration when configured.

Recommended (SoapySDR):

```bash
./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

stdin example (RTL-SDR):

```bash
rtl_sdr -g 48 -f 100900000 -s 2048000 - | ./target/release/novasdr-server -c config/config.json -r config/receivers.json
```

<details>
<summary><strong>Common build issues</strong></summary>

### `frontend/` build fails

- Ensure a supported Node.js version is installed.
- Delete `frontend/node_modules/` and rerun `npm install`.

### WebSockets do not connect behind a reverse proxy

See [Operations](OPERATIONS.md) for the required proxy headers.

</details>
