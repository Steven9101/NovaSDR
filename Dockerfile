# Multi-stage Dockerfile for NovaSDR
# Builds frontend, Rust backend with SoapySDR and OpenCL support

# Stage 1: Build the frontend
FROM node:20-slim AS frontend-builder

WORKDIR /build

# Copy frontend package files
COPY frontend/package*.json ./

# Install dependencies
RUN npm ci

# Copy frontend source
COPY frontend/ ./

# Build the frontend
RUN npm run build

# Stage 2: Build the Rust backend
FROM rustlang/rust:nightly-bookworm-slim AS backend-builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    clang \
    libclang-dev \
    swig \
    python3 \
    python3-dev \
    python3-numpy \
    ocl-icd-opencl-dev \
    libclfft-dev \
    libusb-1.0-0-dev \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy Cargo workspace files
COPY Cargo.toml ./
COPY crates/ ./crates/

# Build SoapySDR from source (for better compatibility)
RUN git clone https://github.com/pothosware/SoapySDR.git /tmp/SoapySDR && \
    cd /tmp/SoapySDR && \
    mkdir build && cd build && \
    cmake .. && \
    make -j$(nproc) && \
    make install && \
    ldconfig && \
    rm -rf /tmp/SoapySDR

# Build SoapyRTLSDR (RTL-SDR support)
RUN apt-get update && apt-get install -y --no-install-recommends rtl-sdr librtlsdr-dev && \
    git clone https://github.com/pothosware/SoapyRTLSDR.git /tmp/SoapyRTLSDR && \
    cd /tmp/SoapyRTLSDR && \
    mkdir build && cd build && \
    cmake .. && \
    make -j$(nproc) && \
    make install && \
    rm -rf /tmp/SoapyRTLSDR && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Build the Rust backend with SoapySDR and clFFT support
RUN cargo build --release --features "soapysdr,clfft" -p novasdr-server

# Build the ws_probe utility
RUN cargo build --release -p ws_probe

# Stage 3: Final runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    libusb-1.0-0 \
    ocl-icd-libopencl1 \
    libclfft2 \
    rtl-sdr \
    python3 \
    python3-numpy \
    ca-certificates \
    netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*

# Copy SoapySDR runtime from builder
COPY --from=backend-builder /usr/local/lib/libSoapySDR* /usr/local/lib/
COPY --from=backend-builder /usr/local/lib/SoapySDR /usr/local/lib/SoapySDR
COPY --from=backend-builder /usr/local/include/SoapySDR /usr/local/include/SoapySDR
COPY --from=backend-builder /usr/local/bin/SoapySDRUtil /usr/local/bin/

# Update library cache
RUN ldconfig

# Create application directory
WORKDIR /app

# Copy built binaries from backend builder
COPY --from=backend-builder /build/target/release/novasdr-server /app/
COPY --from=backend-builder /build/target/release/ws_probe /app/

# Copy built frontend from frontend builder
COPY --from=frontend-builder /build/dist /app/frontend/dist

# Copy configuration files
COPY config/ /app/config/

# Copy resources (default bands, etc.)
COPY crates/novasdr-server/resources/ /app/resources/

# Create directories for runtime data
RUN mkdir -p /app/logs /app/data

# Expose the default port
EXPOSE 9002

# Set environment variables
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# Run the server
CMD ["/app/novasdr-server", "-c", "/app/config/config.json", "-r", "/app/config/receivers.json"]
