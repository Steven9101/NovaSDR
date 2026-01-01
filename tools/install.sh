#!/usr/bin/env sh
set -eu

NOVA_REPO_DEFAULT="Steven9101/NovaSDR"
NOVA_INSTALL_DIR_DEFAULT="/opt/novasdr"
NOVA_BIN_DIR_DEFAULT="/usr/local/bin"
NOVA_SRC_DIR_DEFAULT="/opt/novasdr-src"

usage() {
  cat <<'EOF'
NovaSDR installer (Linux/macOS)

Quick install:
  curl -fsSL https://novasdr.com/install.sh | sh

Optional env vars:
  NOVA_INSTALL_METHOD=source|deps          (default: source)
  NOVA_REPO=owner/repo                     (default: Steven9101/NovaSDR)
  NOVA_REPO_URL=url-or-path                (override repo clone URL for source mode)
  NOVA_REF=git-ref                          (optional: branch/tag/commit to checkout after clone/update)
  NOVA_SKIP_CLONE=1                        (source mode: use existing checkout; do not git clone/fetch; defaults to current directory)
  NOVA_INSTALL_DIR=/opt/novasdr            (default: /opt/novasdr)
  NOVA_BIN_DIR=/usr/local/bin              (default: /usr/local/bin)
  NOVA_SRC_DIR=/opt/novasdr-src            (default: /opt/novasdr-src; source mode only)
  NOVA_NO_SUDO=1                           (disable sudo usage; requires running as root)
  NOVA_NONINTERACTIVE=1                    (use defaults; no prompts)
  NOVA_RUSTUP_INTERACTIVE=1                (force rustup to prompt; default is auto-yes)

SoapySDR (always from source):
  NOVA_DEVICE=rtlsdr|hackrf|airspy|sdrplay|bladerf|limesdr|uhd|all|skip
  NOVA_RTLSDR_V4=1                         (print RTL-SDR v4 driver rebuild steps for apt systems)

Frontend:
  NOVA_FRONTEND=install|build|skip         (default: install)

OpenCL:
  NOVA_OPENCL=install|skip                 (default: install)

clFFT (only needed when building with --features clfft):
  NOVA_CLFFT=install|skip                  (default: install; uses your package manager when possible)

VkFFT (Vulkan; only needed when building with --features vkfft):
  NOVA_VKFFT=install|skip                  (default: skip)

SDRplay API (proprietary):
  NOVA_SDRPLAY_API=install|skip            (default: skip; interactive install only)

Rust toolchain:
  NOVA_RUST=install|skip                   (default: install in source mode; skip otherwise)

Notes:
  - Windows is not supported by this script.
  - This script never runs destructive SDR driver purge steps automatically.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --skip-clone)
      export NOVA_SKIP_CLONE=1
      shift
      ;;
    *)
      err "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

is_tty=0
if [ -t 1 ]; then is_tty=1; fi

can_prompt=0
if [ "${NOVA_NONINTERACTIVE:-}" != "1" ]; then
  if [ -t 1 ] && [ -r /dev/tty ]; then
    can_prompt=1
  fi
fi

supports_color=0
if [ "$is_tty" -eq 1 ] && command -v tput >/dev/null 2>&1; then
  if [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then supports_color=1; fi
fi

if [ "$supports_color" -eq 1 ]; then
  c_reset="$(printf '\033[0m')"
  c_dim="$(printf '\033[2m')"
  c_bold="$(printf '\033[1m')"
  c_red="$(printf '\033[31m')"
  c_green="$(printf '\033[32m')"
  c_yellow="$(printf '\033[33m')"
  c_blue="$(printf '\033[34m')"
  c_magenta="$(printf '\033[35m')"
  c_cyan="$(printf '\033[36m')"
else
  c_reset=""; c_dim=""; c_bold=""; c_red=""; c_green=""; c_yellow=""; c_blue=""; c_magenta=""; c_cyan=""
fi

log() { printf '%s\n' "$*"; }
info() { printf '%s%s%s\n' "$c_cyan" "$*" "$c_reset"; }
ok() { printf '%s%s%s\n' "$c_green" "$*" "$c_reset"; }
warn() { printf '%s%s%s\n' "$c_yellow" "$*" "$c_reset"; }
err() { printf '%s%s%s\n' "$c_red" "$*" "$c_reset" >&2; }

ui_mark() {
  case "$1" in
    ok) printf '%s[%sOK%s]%s' "$c_dim" "$c_green" "$c_dim" "$c_reset" ;;
    warn) printf '%s[%s!!%s]%s' "$c_dim" "$c_yellow" "$c_dim" "$c_reset" ;;
    err) printf '%s[%sXX%s]%s' "$c_dim" "$c_red" "$c_dim" "$c_reset" ;;
    info) printf '%s[%s..%s]%s' "$c_dim" "$c_cyan" "$c_dim" "$c_reset" ;;
    *) printf '%s[..]%s' "$c_dim" "$c_reset" ;;
  esac
}

ui_banner() {
  if [ "$is_tty" -ne 1 ]; then
    log "NovaSDR installer"
    return
  fi
  printf '%s\n' "${c_magenta}${c_bold}==================================================${c_reset}"
  printf '%s\n' "${c_magenta}${c_bold}  NovaSDR installer${c_reset} ${c_dim}(Linux/macOS)${c_reset}"
  printf '%s\n' "${c_magenta}${c_bold}==================================================${c_reset}"
}

hr() {
  if [ "$is_tty" -eq 1 ] && command -v tput >/dev/null 2>&1; then
    cols="$(tput cols 2>/dev/null || echo 80)"
  else
    cols=80
  fi
  i=0
  line=""
  while [ "$i" -lt "$cols" ]; do line="${line}-"; i=$((i + 1)); done
  printf '%s%s%s\n' "$c_dim" "$line" "$c_reset"
}

headline() {
  hr
  printf '%s%s%s\n' "$c_bold" "$1" "$c_reset"
  hr
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    err "$(ui_mark err) Missing required command: $1"
    exit 1
  fi
}

run() {
  if [ "$is_tty" -eq 1 ]; then
    printf '%s+ %s%s\n' "$c_dim" "$*" "$c_reset"
  else
    printf '+ %s\n' "$*"
  fi
  "$@"
}

step() {
  title="$1"
  shift
  printf '%s %s\n' "$(ui_mark info)" "$title"
  "$@"
  printf '%s %s\n' "$(ui_mark ok)" "$title"
}

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  linux) ;;
  darwin) ;;
  *)
    err "Unsupported OS: $os"
    err "This installer supports Linux and macOS."
    exit 1
    ;;
esac

case "$arch" in
  x86_64|amd64) arch_norm="x86_64" ;;
  aarch64|arm64) arch_norm="aarch64" ;;
  armv7l) arch_norm="armv7" ;;
  *)
    err "Unsupported CPU architecture: $arch"
    err "Supported: x86_64, aarch64, armv7l"
    exit 1
    ;;
esac

SUDO=""
if [ "${NOVA_NO_SUDO:-}" = "1" ]; then
  SUDO=""
else
  if [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
      SUDO="sudo"
    else
      err "This installer needs root privileges (sudo not found). Re-run as root or install sudo."
      exit 1
    fi
  fi
fi

detect_pm() {
  if [ "$os" = "darwin" ]; then
    if command -v brew >/dev/null 2>&1; then echo "brew"; return; fi
    echo "none"
    return
  fi

  if command -v apt-get >/dev/null 2>&1; then echo "apt"; return; fi
  if command -v dnf >/dev/null 2>&1; then echo "dnf"; return; fi
  if command -v yum >/dev/null 2>&1; then echo "yum"; return; fi
  if command -v pacman >/dev/null 2>&1; then echo "pacman"; return; fi
  if command -v zypper >/dev/null 2>&1; then echo "zypper"; return; fi
  if command -v apk >/dev/null 2>&1; then echo "apk"; return; fi
  echo "none"
}

pm="$(detect_pm)"

cpu_count() {
  if command -v nproc >/dev/null 2>&1; then nproc; return; fi
  if command -v getconf >/dev/null 2>&1; then getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1; return; fi
  if [ "$os" = "darwin" ] && command -v sysctl >/dev/null 2>&1; then sysctl -n hw.ncpu 2>/dev/null || echo 1; return; fi
  echo 1
}

prompt_yes_no() {
  if [ "$can_prompt" -ne 1 ]; then
    if [ "${2:-}" = "yes" ]; then return 0; fi
    return 1
  fi
  while :; do
    printf "%s [%s/%s] " "$1" "y" "n" >&2
    read -r ans </dev/tty
    case "$(printf '%s' "$ans" | tr '[:upper:]' '[:lower:]' | tr -d ' ')" in
      y|yes) return 0 ;;
      n|no) return 1 ;;
      *) ;;
    esac
  done
}

prompt_select() {
  title="$1"
  default_idx="$2"
  shift 2

  nth_arg() {
    want="$1"
    shift
    i=1
    for v in "$@"; do
      if [ "$i" -eq "$want" ]; then
        printf '%s\n' "$v"
        return 0
      fi
      i=$((i + 1))
    done
    return 1
  }

  if [ "$can_prompt" -ne 1 ]; then
    nth_arg "$default_idx" "$@" || return 1
    return
  fi

  headline "$title" >&2
  i=1
  for opt in "$@"; do
    printf "  %s) %s\n" "$i" "$opt" >&2
    i=$((i + 1))
  done

  while :; do
    printf "%sSelect (default %s):%s " "$c_bold" "$default_idx" "$c_reset" >&2
    read -r raw </dev/tty
    raw="$(printf '%s' "$raw" | tr -d ' ')"
    if [ -z "$raw" ]; then raw="$default_idx"; fi
    case "$raw" in
      ''|*[!0-9]*) ;;
      *)
        idx="$raw"
        if [ "$idx" -ge 1 ] && [ "$idx" -le "$#" ]; then
          nth_arg "$idx" "$@" || return 1
          return
        fi
        ;;
    esac
  done
}

install_packages_common() {
  case "$pm" in
    apt)
      step "apt-get update" run $SUDO apt-get update
      step "Install base tools" run $SUDO apt-get install -y --no-install-recommends ca-certificates curl tar git
      ;;
    dnf)
      step "Install base tools" run $SUDO dnf -y install ca-certificates curl tar git
      ;;
    yum)
      step "Install base tools" run $SUDO yum -y install ca-certificates curl tar git
      ;;
    pacman)
      step "Install base tools" run $SUDO pacman -Sy --noconfirm --needed ca-certificates curl tar git
      ;;
    zypper)
      step "zypper refresh" run $SUDO zypper --non-interactive refresh
      step "Install base tools" run $SUDO zypper --non-interactive install -y ca-certificates curl tar git
      ;;
    apk)
      step "Install base tools" run $SUDO apk add --no-cache ca-certificates curl tar git
      warn "Alpine support is limited (musl): you may need extra work to build and run NovaSDR."
      ;;
    brew)
      step "brew update" run brew update
      step "Install base tools" run brew install curl tar git
      ;;
    none)
      warn "No supported package manager detected; skipping package install."
      ;;
  esac
}

install_packages_build_tools() {
  headline "Dependencies: build tools"

  case "$pm" in
    apt)
      step "apt-get update" run $SUDO apt-get update
      step "Install build tools" run $SUDO apt-get install -y --no-install-recommends \
        build-essential cmake pkg-config \
        clang libclang-dev \
        swig \
        python3 python3-dev python3-numpy
      ;;
    dnf)
      step "Install build tools" run $SUDO dnf -y install \
        gcc gcc-c++ make cmake pkgconf-pkg-config \
        clang llvm-devel libclang-devel \
        swig \
        python3 python3-devel python3-numpy
      ;;
    yum)
      step "Install build tools" run $SUDO yum -y install \
        gcc gcc-c++ make cmake pkgconfig \
        clang llvm-devel libclang-devel \
        swig \
        python3 python3-devel
      ;;
    pacman)
      step "Install build tools" run $SUDO pacman -Sy --noconfirm --needed \
        base-devel cmake pkgconf \
        clang llvm libclang \
        swig \
        python python-numpy
      ;;
    zypper)
      step "Install build tools" run $SUDO zypper --non-interactive install -y \
        gcc-c++ make cmake pkg-config \
        clang llvm llvm-devel libclang-devel \
        swig \
        python3 python3-devel python3-numpy
      ;;
    brew)
      step "Install build tools" run brew install cmake pkg-config swig python llvm
      ;;
    apk)
      warn "Alpine: install build tools manually or use a glibc-based distro/container."
      ;;
    none)
      warn "No supported package manager detected; install build tools manually."
      ;;
  esac
}

install_packages_node() {
  headline "Dependencies: Node.js (frontend build)"

  case "$pm" in
    apt)
      warn "Node.js packages vary by distro release."
      warn "Recommended: install Node.js LTS from your distro, or from nodejs.org."
      run $SUDO apt-get install -y --no-install-recommends nodejs npm || true
      ;;
    dnf)
      run $SUDO dnf -y install nodejs npm || true
      ;;
    yum)
      run $SUDO yum -y install nodejs npm || true
      ;;
    pacman)
      run $SUDO pacman -Sy --noconfirm --needed nodejs npm || true
      ;;
    zypper)
      run $SUDO zypper --non-interactive install -y nodejs npm || true
      ;;
    brew)
      run brew install node
      ;;
    *)
      warn "Install Node.js manually (required for building the frontend)."
      ;;
  esac
}

install_packages_opencl() {
  headline "Dependencies: OpenCL"

  case "$pm" in
    apt)
      step "apt-get update" run $SUDO apt-get update
      step "Install OpenCL headers/runtime" run $SUDO apt-get install -y --no-install-recommends \
        ocl-icd-opencl-dev ocl-icd-libopencl1
      ;;
    dnf)
      step "Install OpenCL headers/runtime" run $SUDO dnf -y install \
        ocl-icd ocl-icd-devel
      ;;
    yum)
      step "Install OpenCL headers/runtime" run $SUDO yum -y install \
        ocl-icd ocl-icd-devel
      ;;
    pacman)
      step "Install OpenCL headers/runtime" run $SUDO pacman -Sy --noconfirm --needed \
        ocl-icd opencl-headers
      ;;
    zypper)
      step "Install OpenCL headers/runtime" run $SUDO zypper --non-interactive install -y \
        OpenCL-Headers ocl-icd-devel
      ;;
    brew)
      warn "OpenCL is provided by macOS; NovaSDR uses it only if available at runtime."
      ;;
    apk)
      warn "Alpine: install OpenCL manually or use a glibc-based distro/container."
      ;;
    none)
      warn "No supported package manager detected; install OpenCL manually."
      ;;
  esac
}

install_packages_clfft() {
  headline "Dependencies: clFFT (OpenCL)"

  case "$pm" in
    apt)
      step "Install clFFT (apt)" run $SUDO apt-get install -y --no-install-recommends libclfft-dev || true
      ;;
    dnf|yum)
      warn "clFFT package names vary by distro; attempting a best-effort install."
      run $SUDO "$pm" -y install clfft clfft-devel 2>/dev/null || true
      ;;
    zypper)
      warn "clFFT package names vary by distro; attempting a best-effort install."
      run $SUDO zypper --non-interactive install -y clfft clfft-devel 2>/dev/null || true
      ;;
    pacman)
      warn "Arch: clFFT may not be available in official repos. Install it manually if you want --features clfft."
      ;;
    apk)
      warn "Alpine: clFFT packages may not be available. Install clFFT manually if you want --features clfft."
      ;;
    brew)
      warn "macOS: clFFT installation varies; install clFFT manually if you want --features clfft."
      ;;
    none)
      warn "No supported package manager detected; install clFFT manually if you want --features clfft."
      ;;
  esac
}

vkfft_headers_are_available() {
  vkfft_layout_ok() {
    include_dir="$1"
    [ -f "$include_dir/vkFFT.h" ] || return 1
    [ -f "$include_dir/vkFFT/vkFFT_Structs/vkFFT_Structs.h" ] || return 1
    return 0
  }

  glslang_ok=1
  for d in /usr/include /usr/local/include; do
    if [ -f "$d/glslang/Include/glslang_c_interface.h" ] || [ -f "$d/glslang_c_interface.h" ]; then
      glslang_ok=0
      break
    fi
  done
  if [ "$glslang_ok" -ne 0 ]; then
    return 1
  fi

  for d in \
    /usr/include/vkfft /usr/include/vkFFT /usr/include/VkFFT /usr/include \
    /usr/local/include/vkfft /usr/local/include/vkFFT /usr/local/include/VkFFT /usr/local/include
  do
    if vkfft_layout_ok "$d"; then
      return 0
    fi
  done

  return 1
}

install_packages_vkfft() {
  headline "Dependencies: VkFFT (Vulkan)"

  case "$pm" in
    apt)
      step "apt-get update" run $SUDO apt-get update
      step "Install Vulkan + glslang + SPIRV-Tools (apt)" run $SUDO apt-get install -y --no-install-recommends \
        libvkfft-dev \
        libvulkan-dev \
        glslang-dev \
        spirv-tools \
        pkg-config \
      || true
      ;;
    dnf|yum)
      warn "Vulkan/glslang package names vary by distro; attempting a best-effort install."
      run $SUDO "$pm" -y install \
        vulkan-loader-devel vulkan-headers glslang spirv-tools pkgconf-pkg-config 2>/dev/null || true
      ;;
    zypper)
      warn "Vulkan/glslang package names vary by distro; attempting a best-effort install."
      run $SUDO zypper --non-interactive install -y \
        vulkan-devel glslang spirv-tools pkg-config 2>/dev/null || true
      ;;
    pacman)
      run $SUDO pacman -Sy --noconfirm --needed vulkan-headers vulkan-tools glslang spirv-tools pkgconf || true
      ;;
    brew)
      warn "macOS: VkFFT support is Linux-only in NovaSDR currently."
      ;;
    apk)
      warn "Alpine: VkFFT/Vulkan packages may not be available. Install Vulkan + glslang manually if you want --features vkfft."
      ;;
    none)
      warn "No supported package manager detected; install Vulkan + glslang manually if you want --features vkfft."
      ;;
  esac
}

clfft_is_available() {
  if command -v ldconfig >/dev/null 2>&1; then
    ldconfig -p 2>/dev/null | tr '[:upper:]' '[:lower:]' | grep -q 'clfft' && return 0
  fi

  for d in /usr/lib /usr/lib64 /usr/local/lib /usr/local/lib64 /lib /lib64; do
    if ls "$d"/libclFFT.so* >/dev/null 2>&1; then return 0; fi
    if ls "$d"/libclfft.so* >/dev/null 2>&1; then return 0; fi
  done

  return 1
}

ensure_clfft_available() {
  if clfft_is_available; then
    return 0
  fi
  err "clFFT library not found after installation attempt."
  err "Install clFFT using your distro packages (Debian/Ubuntu: libclfft-dev),"
  err "or re-run with clFFT disabled."
  exit 1
}

maybe_set_libclang_path() {
  if [ -n "${LIBCLANG_PATH:-}" ]; then
    return
  fi

  if [ "$os" = "darwin" ] && command -v brew >/dev/null 2>&1; then
    p="$(brew --prefix llvm 2>/dev/null || true)"
    if [ -n "$p" ] && [ -d "$p/lib" ]; then
      export LIBCLANG_PATH="$p/lib"
      info "LIBCLANG_PATH set to: $LIBCLANG_PATH"
      return
    fi
  fi

  for d in \
    /usr/lib/llvm-18/lib \
    /usr/lib/llvm-17/lib \
    /usr/lib/llvm-16/lib \
    /usr/lib/llvm-15/lib \
    /usr/lib/llvm-14/lib \
    /usr/local/opt/llvm/lib \
    /opt/homebrew/opt/llvm/lib \
    /usr/lib \
    /usr/local/lib
  do
    # shellcheck disable=SC2086
    if ls $d/libclang.so* >/dev/null 2>&1; then
      export LIBCLANG_PATH="$d"
      info "LIBCLANG_PATH set to: $LIBCLANG_PATH"
      return
    fi
    # shellcheck disable=SC2086
    if ls $d/libclang.dylib >/dev/null 2>&1; then
      export LIBCLANG_PATH="$d"
      info "LIBCLANG_PATH set to: $LIBCLANG_PATH"
      return
    fi
  done
}

install_rustup() {
  headline "Rust toolchain"

  if command -v cargo >/dev/null 2>&1; then
    ok "Rust already installed: $(cargo --version 2>/dev/null || echo cargo)"
    return
  fi

  if [ "${NOVA_RUSTUP_INTERACTIVE:-}" = "1" ] && [ "$can_prompt" -eq 1 ]; then
    info "Installing Rust with rustup (interactive)."
    info "You may be asked to confirm installation options."
    run sh -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  else
    warn "Installing Rust with rustup (auto-yes)."
    step "Install rustup" run sh -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
  fi

  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1090
    . "$HOME/.cargo/env"
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    err "Rust installation finished but cargo is not on PATH."
    err "Run:"
    err "  . \"$HOME/.cargo/env\""
    err "Then re-run this installer."
    exit 1
  fi

  ok "Rust installed: $(cargo --version)"
}

npm_install_and_build_frontend() {
  headline "Frontend build"

  if [ "${NOVA_FRONTEND:-}" != "build" ]; then
    return 0
  fi

  if ! command -v npm >/dev/null 2>&1; then
    warn "npm not found; skipping frontend build."
    return 0
  fi

  if [ ! -d "frontend" ]; then
    warn "frontend/ directory not found; skipping frontend build."
    return 0
  fi

  if [ ! -f "frontend/package.json" ]; then
    warn "frontend/package.json not found; skipping frontend build."
    return 0
  fi

  if [ -f "frontend/package-lock.json" ]; then
    (cd frontend && run npm ci && run npm run build)
    return 0
  fi

  warn "frontend/package-lock.json not found; falling back to 'npm install' (non-reproducible)."
  warn "Commit a package-lock.json to enable deterministic installs via 'npm ci'."
  (cd frontend && run npm install --no-audit --no-fund && run npm run build)
}

install_frontend_dist_into_install_dir() {
  src_dir="$1"
  install_dir="$2"

  if [ ! -d "$src_dir/frontend/dist" ]; then
    err "frontend/dist not found in source tree."
    err "Run:"
    err "  git submodule update --init --recursive"
    err "Or set NOVA_FRONTEND=build (requires npm), or NOVA_FRONTEND=skip."
    exit 1
  fi

  run $SUDO mkdir -p "$install_dir/frontend"
  run $SUDO rm -rf "$install_dir/frontend/dist"
  step "Install frontend/dist" run $SUDO cp -R "$src_dir/frontend/dist" "$install_dir/frontend/"
}

tmpdir=""
cleanup() {
  if [ -n "$tmpdir" ] && [ -d "$tmpdir" ]; then
    rm -rf "$tmpdir" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

mk_tmpdir() {
  if [ -n "$tmpdir" ]; then return; fi
  tmpdir="$(mktemp -d 2>/dev/null || mktemp -d -t novasdr)"
}

install_packages_device_deps() {
  device="$1"
  headline "Dependencies: device ($device)"

  case "$device" in
    rtlsdr)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends rtl-sdr librtlsdr-dev libusb-1.0-0-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install rtl-sdr rtl-sdr-devel libusb1-devel || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed rtl-sdr libusb || true ;;
        zypper) run $SUDO zypper --non-interactive install -y rtl-sdr rtl-sdr-devel libusb-1_0-devel || true ;;
        brew) warn "Homebrew: install librtlsdr (rtl-sdr) if needed."; run brew install rtl-sdr || true ;;
        *) warn "Install RTL-SDR development packages manually." ;;
      esac
      ;;
    hackrf)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends hackrf libhackrf-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install hackrf hackrf-devel || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed hackrf || true ;;
        zypper) run $SUDO zypper --non-interactive install -y hackrf hackrf-devel || true ;;
        brew) run brew install hackrf || true ;;
        *) warn "Install HackRF development packages manually." ;;
      esac
      ;;
    airspy)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends airspy libairspy-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install airspy airspy-devel || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed airspy || true ;;
        zypper) run $SUDO zypper --non-interactive install -y airspy airspy-devel || true ;;
        brew) warn "Homebrew: install libairspy if available."; run brew install airspy || true ;;
        *) warn "Install Airspy development packages manually." ;;
      esac
      ;;
    bladerf)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends bladerf libbladerf-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install bladerf bladerf-devel || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed bladerf || true ;;
        zypper) run $SUDO zypper --non-interactive install -y bladerf bladerf-devel || true ;;
        brew) run brew install bladerf || true ;;
        *) warn "Install bladeRF development packages manually." ;;
      esac
      ;;
    limesdr)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends limesuite liblimesuite-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install limesuite limesuite-devel 2>/dev/null || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed limesuite || true ;;
        zypper) run $SUDO zypper --non-interactive install -y limesuite liblimesuite-devel 2>/dev/null || true ;;
        brew) run brew install limesuite || true ;;
        *) warn "Install LimeSuite (limesdr) development packages manually." ;;
      esac
      ;;
    uhd)
      case "$pm" in
        apt) run $SUDO apt-get install -y --no-install-recommends uhd-host libuhd-dev || true ;;
        dnf|yum) run $SUDO "$pm" -y install uhd uhd-devel 2>/dev/null || true ;;
        pacman) run $SUDO pacman -Sy --noconfirm --needed uhd || true ;;
        zypper) run $SUDO zypper --non-interactive install -y uhd libuhd-devel 2>/dev/null || true ;;
        brew) run brew install uhd || true ;;
        *) warn "Install UHD (usrp) development packages manually." ;;
      esac
      ;;
    sdrplay)
      warn "SDRplay requires the proprietary SDRplay API to be installed (see SDRplay section)."
      ;;
    *)
      ;;
  esac
}

install_sdrplay_api() {
  headline "SDRplay API (proprietary)"

  if [ "$os" != "linux" ]; then
    err "SDRplay API automatic install is supported only on Linux by this script."
    exit 1
  fi

  if [ "$can_prompt" -ne 1 ]; then
    err "SDRplay API install requires an interactive terminal for license acceptance."
    exit 1
  fi

  need_cmd curl

  url="https://www.sdrplay.com/software/SDRplay_RSP_API-Linux-3.15.2.run"
  mk_tmpdir
  installer="$tmpdir/SDRplay_RSP_API-Linux-3.15.2.run"

  warn "This downloads and runs a proprietary installer from SDRplay."
  warn "You must accept the license terms yourself to use SDRplay hardware legally."
  warn "During install you will be prompted to view the license and accept it."
  warn "Common flow: press Enter, then 'q', then 'y', then 'y'."

  step "Download SDRplay API installer" run curl -fL -o "$installer" "$url"
  run chmod +x "$installer"
  # The installer reads from stdin. When this script is invoked via `curl ... | sh`,
  # stdin is a pipe (not a tty), and license acceptance becomes impossible. Always
  # attach the installer to /dev/tty so the user can interact with it.
  step "Run SDRplay API installer (interactive)" run $SUDO sh -c "\"$installer\" </dev/tty"

  warn "SDRplay API install finished. Reboot may be required for the service/device to be available."
  warn "Service control: sudo systemctl start sdrplay | sudo systemctl stop sdrplay"
}

cmake_build_install() {
  src="$1"
  jobs="$(cpu_count)"
  run mkdir -p "$src/build"
  (cd "$src/build" && run cmake .. && run make -j"$jobs" && run $SUDO make install)
  if [ "$os" = "linux" ] && command -v ldconfig >/dev/null 2>&1; then
    run $SUDO ldconfig
  fi
}

build_and_install_soapysdr_source() {
  headline "SoapySDR: build from source"

  need_cmd git
  need_cmd cmake
  need_cmd make

  mk_tmpdir
  src="$tmpdir/SoapySDR"
  step "Clone SoapySDR" run git clone --depth 1 https://github.com/pothosware/SoapySDR.git "$src"
  step "Build + install SoapySDR" cmake_build_install "$src"

  if command -v SoapySDRUtil >/dev/null 2>&1; then
    ok "SoapySDR installed: $(SoapySDRUtil --info 2>/dev/null | head -n 1 || echo SoapySDRUtil)"
  else
    warn "SoapySDR installed, but SoapySDRUtil is not on PATH."
  fi
}

build_and_install_soapy_module_source() {
  headline "SoapySDR device module: build from source"

  need_cmd git
  need_cmd cmake
  need_cmd make

  device="${NOVA_DEVICE:-}"
  if [ -z "$device" ]; then
    choice="$(prompt_select "Select your SDR device" 1 \
      "RTL-SDR" "HackRF" "Airspy" "SDRplay" "bladeRF" "LimeSDR" "USRP (UHD)" "All" "Skip")"
    case "$choice" in
      "RTL-SDR") device="rtlsdr" ;;
      "HackRF") device="hackrf" ;;
      "Airspy") device="airspy" ;;
      "SDRplay") device="sdrplay" ;;
      "bladeRF") device="bladerf" ;;
      "LimeSDR") device="limesdr" ;;
      "USRP (UHD)") device="uhd" ;;
      "All") device="all" ;;
      "Skip") device="skip" ;;
    esac
  fi

  if [ "$device" = "skip" ]; then
    warn "Skipping device module build."
    return
  fi

  install_one() {
    d="$1"
    install_packages_device_deps "$d"
    mk_tmpdir

    repo=""
    case "$d" in
      rtlsdr) repo="SoapyRTLSDR" ;;
      hackrf) repo="SoapyHackRF" ;;
      airspy) repo="SoapyAirspy" ;;
      bladerf) repo="SoapyBladeRF" ;;
      limesdr) repo="SoapyLMS7" ;;
      uhd) repo="SoapyUHD" ;;
      sdrplay) repo="SoapySDRPlay" ;;
      *) repo="" ;;
    esac

    if [ -z "$repo" ]; then
      warn "Unsupported device selection: $d"
      return 1
    fi

    if [ "$d" = "sdrplay" ]; then
      warn "SDRplay: SoapySDRPlay requires the proprietary SDRplay API."
      if [ "${NOVA_SDRPLAY_API:-skip}" = "install" ]; then
        install_sdrplay_api
      else
        if prompt_yes_no "Install the SDRplay API now? (interactive license acceptance required)" "no"; then
          install_sdrplay_api
        fi
      fi
    fi

    src="$tmpdir/$repo"
    step "Clone $repo" run git clone --depth 1 "https://github.com/pothosware/${repo}.git" "$src"
    step "Build + install $repo" cmake_build_install "$src"
    ok "Soapy module build complete: $repo"
    return 0
  }

  failures=0
  if [ "$device" = "all" ]; then
    for d in rtlsdr hackrf airspy sdrplay bladerf limesdr uhd; do
      if ! install_one "$d"; then failures=$((failures + 1)); fi
    done
  else
    if ! install_one "$device"; then failures=$((failures + 1)); fi
  fi

  if [ "$failures" -ne 0 ]; then
    warn "Some SoapySDR modules failed to build (${failures}). Check the logs above for missing dependencies."
  fi
}

print_rtlsdr_v4_instructions_apt() {
  headline "RTL-SDR v4: driver rebuild (advanced reference)"
  cat <<'EOF'
Purge the previous driver:
  sudo apt purge ^librtlsdr
  sudo rm -rvf /usr/lib/librtlsdr* /usr/include/rtl-sdr* /usr/local/lib/librtlsdr* /usr/local/include/rtl-sdr* /usr/local/include/rtl_* /usr/local/bin/rtl_*

Install build deps:
  sudo apt-get install -y libusb-1.0-0-dev git cmake pkg-config build-essential

Install the latest drivers:
  git clone https://github.com/osmocom/rtl-sdr
  cd rtl-sdr
  mkdir build
  cd build
  cmake ../ -DINSTALL_UDEV_RULES=ON
  make
  sudo make install
  sudo cp ../rtl-sdr.rules /etc/udev/rules.d/
  sudo ldconfig

Blacklist the DVB-T drivers:
  echo 'blacklist dvb_usb_rtl28xxu' | sudo tee --append /etc/modprobe.d/blacklist-dvb_usb_rtl28xxu.conf

Reboot after installing new kernel modules.
EOF
}

build_from_source() {
  headline "NovaSDR: build from source"

  need_cmd git

  skip_clone="${NOVA_SKIP_CLONE:-0}"
  src_dir="${NOVA_SRC_DIR:-$NOVA_SRC_DIR_DEFAULT}"
  if [ "$skip_clone" = "1" ] && [ -z "${NOVA_SRC_DIR:-}" ]; then
    src_dir="$(pwd)"
  fi
  repo="${NOVA_REPO:-$NOVA_REPO_DEFAULT}"
  repo_url="${NOVA_REPO_URL:-}"
  if [ -z "$repo_url" ]; then
    repo_url="https://github.com/${repo}.git"
  fi

  if [ "${NOVA_RUST:-install}" = "install" ]; then
    install_rustup
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    err "cargo not found; Rust is required for source builds."
    exit 1
  fi

  if [ "$skip_clone" = "1" ]; then
    if [ ! -f "$src_dir/Cargo.toml" ]; then
      err "NOVA_SKIP_CLONE=1 set, but NOVA_SRC_DIR does not look like a NovaSDR checkout: $src_dir"
      err "Expected: $src_dir/Cargo.toml"
      exit 1
    fi
    info "Using existing checkout (skip clone): $src_dir"
  else
    run $SUDO mkdir -p "$src_dir"
    run $SUDO chown "$(id -u)":"$(id -g)" "$src_dir" 2>/dev/null || true
    if [ -d "$src_dir/.git" ]; then
      info "Updating existing checkout: $src_dir"
      if [ -n "${NOVA_REPO_URL:-}" ]; then
        warn "NOVA_REPO_URL is set; using existing checkout without fetch/pull."
      else
        (cd "$src_dir" && run git fetch --all && run git pull --ff-only)
      fi
    else
      info "Cloning: $repo_url"
      run git clone --recurse-submodules "$repo_url" "$src_dir"
    fi
  fi

  if [ -n "${NOVA_REF:-}" ]; then
    (cd "$src_dir" && run git checkout "$NOVA_REF")
  fi

  if [ -f "$src_dir/.gitmodules" ]; then
    (cd "$src_dir" && run git submodule update --init --recursive)
  fi

  features=""

  if prompt_yes_no "Build with SoapySDR support (--features soapysdr)?" "yes"; then
    features="soapysdr"
  fi

  clfft_default="no"
  if [ "$os" = "linux" ]; then
    clfft_default="yes"
  fi
  if prompt_yes_no "Enable OpenCL clFFT acceleration (--features clfft)?" "$clfft_default"; then
    clfft_mode="${NOVA_CLFFT:-install}"
    if [ "${NOVA_NONINTERACTIVE:-}" = "1" ] && [ -z "${NOVA_CLFFT:-}" ]; then
      clfft_mode="install"
    fi

    if [ "$opencl_mode" = "skip" ]; then
      err "clfft requires OpenCL headers/runtime; set NOVA_OPENCL=install."
      exit 1
    fi

    if [ "$clfft_mode" = "install" ]; then
      install_packages_clfft
      ensure_clfft_available
    else
      warn "Skipping clFFT installation (NOVA_CLFFT=skip). Build may fail if clFFT is not installed."
    fi

    if [ -n "$features" ]; then features="${features},clfft"; else features="clfft"; fi
  fi

  vkfft_default="no"
  if [ "$os" = "linux" ]; then
    vkfft_default="no"
  fi
  if prompt_yes_no "Enable Vulkan VkFFT acceleration (--features vkfft)?" "$vkfft_default"; then
    if [ "$os" != "linux" ]; then
      err "vkfft is supported only on Linux."
      exit 1
    fi

    # If the user opts into the vkfft feature, default to installing dependencies.
    vkfft_mode="${NOVA_VKFFT:-install}"
    if [ "${NOVA_NONINTERACTIVE:-}" = "1" ] && [ -z "${NOVA_VKFFT:-}" ]; then
      vkfft_mode="skip"
    fi

    if [ "$vkfft_mode" = "install" ]; then
      install_packages_vkfft
      if ! vkfft_headers_are_available; then
        err "VkFFT headers not found after installation attempt (vkFFT.h)."
        err "glslang headers not found after installation attempt (glslang_c_interface.h)."
        err "On Debian/Ubuntu, install: libvkfft-dev glslang-dev"
        exit 1
      fi
    else
      warn "Skipping VkFFT dependency installation (NOVA_VKFFT=skip). Build may fail if Vulkan/glslang are not installed."
    fi

    if [ -n "$features" ]; then features="${features},vkfft"; else features="vkfft"; fi
  fi

  jobs="$(cpu_count)"
  maybe_set_libclang_path

  if [ -n "$features" ]; then
    (cd "$src_dir" && run cargo build -p novasdr-server --release --features "$features")
  else
    (cd "$src_dir" && run cargo build -p novasdr-server --release)
  fi

  install_dir="${NOVA_INSTALL_DIR:-$NOVA_INSTALL_DIR_DEFAULT}"
  bin_dir="${NOVA_BIN_DIR:-$NOVA_BIN_DIR_DEFAULT}"

  run $SUDO mkdir -p "$install_dir"
  run $SUDO cp "$src_dir/target/release/novasdr-server" "$install_dir/novasdr-server"
  run $SUDO chmod +x "$install_dir/novasdr-server"

  frontend_mode="${NOVA_FRONTEND:-install}"

  case "$frontend_mode" in
    skip)
      info "Skipping frontend install (NOVA_FRONTEND=skip)."
      ;;
    install)
      install_frontend_dist_into_install_dir "$src_dir" "$install_dir"
      ;;
    build)
      if prompt_yes_no "Install Node.js + npm (needed for frontend build)?" "yes"; then
        install_packages_node
      fi
      (cd "$src_dir" && npm_install_and_build_frontend)
      install_frontend_dist_into_install_dir "$src_dir" "$install_dir"
      ;;
    *)
      err "Invalid NOVA_FRONTEND: $frontend_mode (expected install|build|skip)"
      exit 1
      ;;
  esac

  run $SUDO mkdir -p "$bin_dir"
  run $SUDO ln -sf "$install_dir/novasdr-server" "$bin_dir/novasdr-server"

  ok "Built and installed to: $install_dir"
  ok "Binary:               $bin_dir/novasdr-server"

  info "Build jobs: ${jobs}"
}

post_install_notes() {
  headline "Next steps"
  cat <<EOF
${c_bold}1) Run setup wizard${c_reset}
  novasdr-server setup -c ./config/config.json -r ./config/receivers.json

${c_bold}2) Start server${c_reset}
  novasdr-server -c ./config/config.json -r ./config/receivers.json

${c_bold}3) Verify SoapySDR sees your device${c_reset}
  SoapySDRUtil --find
EOF
}

ui_banner
log "${c_dim}Platform:${c_reset} ${os}/${arch_norm}"
log "${c_dim}Package manager:${c_reset} ${pm}"

install_method="${NOVA_INSTALL_METHOD:-source}"
if [ "${NOVA_NONINTERACTIVE:-}" = "1" ] && [ -z "${NOVA_INSTALL_METHOD:-}" ]; then
  install_method="source"
fi

case "$install_method" in
  source|deps) ;;
  *)
    err "Invalid NOVA_INSTALL_METHOD: $install_method (expected source|deps)"
    exit 1
    ;;
esac

if [ "$pm" = "none" ]; then
  warn "No supported package manager detected; some steps will be skipped."
fi

if [ "${NOVA_SKIP_CLONE:-0}" = "1" ]; then
  if [ "$install_method" != "source" ]; then
    warn "NOVA_SKIP_CLONE=1 implies a local source build; forcing NOVA_INSTALL_METHOD=source."
    install_method="source"
  fi
fi

install_packages_common

opencl_mode="${NOVA_OPENCL:-install}"
if [ "${NOVA_NONINTERACTIVE:-}" = "1" ] && [ -z "${NOVA_OPENCL:-}" ]; then
  opencl_mode="install"
fi

install_packages_build_tools

if [ "$opencl_mode" = "install" ]; then
  if prompt_yes_no "Install OpenCL dependencies?" "yes"; then
    install_packages_opencl
  fi
fi

clfft_mode="${NOVA_CLFFT:-install}"
if [ "${NOVA_NONINTERACTIVE:-}" = "1" ] && [ -z "${NOVA_CLFFT:-}" ]; then
  clfft_mode="install"
fi
if [ "$opencl_mode" = "install" ] && [ "$clfft_mode" = "install" ]; then
  if prompt_yes_no "Install clFFT library packages? (required for --features clfft builds)" "yes"; then
    install_packages_clfft
    if [ "$os" = "linux" ] && ! clfft_is_available; then
      warn "clFFT library not found after package install attempt."
      warn "If you plan to run a clFFT-enabled build, install clFFT manually (Debian/Ubuntu: libclfft-dev)."
    fi
  fi
fi

step "Build + install SoapySDR (from source)" build_and_install_soapysdr_source

if prompt_yes_no "Build and install SoapySDR device module(s) from source now?" "yes"; then
  build_and_install_soapy_module_source
fi

if [ "${NOVA_RTLSDR_V4:-}" = "1" ] && [ "$pm" = "apt" ]; then
  warn "RTL-SDR v4 driver rebuild instructions (review carefully):"
  print_rtlsdr_v4_instructions_apt
fi

case "$install_method" in
  source)
    if [ "${NOVA_RUST:-install}" = "install" ]; then
      install_rustup
    fi
    build_from_source
    post_install_notes
    ;;
  deps)
    ok "Dependency install complete."
    ;;
esac
