# Installation

This document describes installing NovaSDR on Linux/macOS using the provided installer script (`tools/install.sh`), or manually.

## Requirements

- Linux or macOS
- SoapySDR (recommended for hardware input)
- A supported SDR device driver (via a SoapySDR module)

OpenCL (clFFT) is part of the recommended build. NovaSDR uses OpenCL only when built with the `clfft` feature and when a compatible OpenCL runtime is available.
When building with `--features clfft`, you also need the `clFFT` library installed (the installer attempts to install it via your package manager; see `NOVA_CLFFT`).

VkFFT (Vulkan) is an optional Linux-only accelerator. NovaSDR uses it only when built with the `vkfft` feature and when a Vulkan-capable driver/stack is available.
When building with `--features vkfft`, you also need VkFFT + `glslang` headers and SPIRV-Tools (the installer can attempt to install them; see `NOVA_VKFFT`).

## One-line installer (Linux/macOS)

```sh
curl -fsSL https://novasdr.com/install.sh | sh
```

The installer:

- builds and installs SoapySDR from source
- optionally installs OpenCL headers/runtime (where available)
- optionally installs `clFFT` packages (required for `--features clfft` builds)
- optionally installs Vulkan/glslang/SPIRV-Tools packages (required for `--features vkfft` builds)
- optionally builds a SoapySDR device module from source
- builds and installs NovaSDR from source
- does not build the frontend unless requested (`NOVA_FRONTEND=build`)

To enable VkFFT builds, answer **yes** when prompted for `--features vkfft`. By default, the installer will attempt to install the required Vulkan/glslang/VkFFT packages when you opt in (override with `NOVA_VKFFT=skip`).

Example (Raspberry Pi 4 / Debian/Raspberry Pi OS):

```sh
curl -fsSL https://novasdr.com/install.sh | NOVA_VKFFT=install sh
```

### Distro packages (manual)

These commands install typical build/runtime dependencies. Exact package names may vary by distro version.

#### Debian/Ubuntu (apt)

```sh
sudo apt-get update && sudo apt-get install -y --no-install-recommends \
  ca-certificates curl tar git \
  build-essential cmake pkg-config \
  clang libclang-dev \
  swig python3 python3-dev python3-numpy \
  nodejs npm \
  ocl-icd-opencl-dev ocl-icd-libopencl1 \
  libclfft-dev \
  libvkfft-dev libvulkan-dev glslang-dev spirv-tools \
  libusb-1.0-0-dev
```

Build SoapySDR from source (manual reference; the installer does this automatically):

```sh
git clone https://github.com/pothosware/SoapySDR.git
cd SoapySDR
mkdir build && cd build
cmake ..
make -j"$(nproc)"
sudo make install
sudo ldconfig
SoapySDRUtil --info
```

Install `clFFT` from your distro packages (required for `--features clfft`).
Example (Debian/Ubuntu): `sudo apt-get install -y --no-install-recommends libclfft-dev`.

#### Fedora (dnf)

Install build/runtime deps (example):

```sh
sudo dnf install -y ca-certificates curl tar git gcc gcc-c++ make cmake pkgconf-pkg-config clang llvm-devel libclang-devel swig python3 python3-devel python3-numpy ocl-icd ocl-icd-devel
```

#### Arch (pacman)

Install build/runtime deps (example):

```sh
sudo pacman -Sy --noconfirm --needed ca-certificates curl tar git base-devel cmake pkgconf clang llvm libclang swig python python-numpy ocl-icd opencl-headers
```

#### openSUSE (zypper)

Install build/runtime deps (example):

```sh
sudo zypper --non-interactive refresh
sudo zypper --non-interactive install -y ca-certificates curl tar git gcc-c++ make cmake pkg-config clang llvm llvm-devel libclang-devel swig python3 python3-devel python3-numpy OpenCL-Headers ocl-icd-devel
```

#### macOS (Homebrew)

Install build deps (example):

```sh
brew install git cmake pkg-config llvm swig python
```

### Non-interactive mode

```sh
curl -fsSL https://novasdr.com/install.sh | NOVA_NONINTERACTIVE=1 sh
```

### Skip building SoapySDR device modules

SoapySDR itself is always built and installed from source by `tools/install.sh`. To skip building any device modules:

```sh
curl -fsSL https://novasdr.com/install.sh | NOVA_DEVICE=skip sh
```

### Build all SoapySDR device modules

```sh
curl -fsSL https://novasdr.com/install.sh | NOVA_DEVICE=all sh
```

### SDRplay API (proprietary)

`SoapySDRPlay` requires SDRplay's proprietary API. The installer can download and run it, but it will not auto-accept the license:

```sh
curl -fsSL https://novasdr.com/install.sh | NOVA_SDRPLAY_API=install sh
```

Note: the SDRplay `.run` installer is interactive and reads from stdin. The NovaSDR installer will attach it to `/dev/tty` so it remains interactive even when invoked via `curl ... | sh`.

### Source mode from a local checkout (repo not published yet)

Use `NOVA_REPO_URL` to point at a local path or alternative git URL:

```sh
curl -fsSL https://novasdr.com/install.sh | \
  NOVA_INSTALL_METHOD=source \
  NOVA_REPO_URL=/path/to/NovaSDR \
  sh
```

If the repo is already cloned to `NOVA_SRC_DIR`, you can skip all git operations:

```sh
curl -fsSL https://novasdr.com/install.sh | \
  NOVA_INSTALL_METHOD=source \
  NOVA_SRC_DIR=/opt/novasdr-src \
  sh -s -- --skip-clone
```

If you are already in your cloned repo directory, you can also omit `NOVA_SRC_DIR`:

```sh
export NOVA_SKIP_CLONE=1
./tools/install.sh
```
