# ESTÁGIO 1: Frontend
FROM node:20-slim AS frontend-builder
WORKDIR /build
COPY frontend/package*.json ./
# Ajuste para resiliência de submódulo
RUN if [ -f package-lock.json ]; then npm ci; else npm install; fi
COPY frontend/ ./
RUN npm run build

# ESTÁGIO 2: Planejamento
FROM rustlang/rust:nightly-bookworm-slim AS planner
WORKDIR /build
RUN cargo install cargo-chef
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ESTÁGIO 3: Backend Builder
FROM rustlang/rust:nightly-bookworm-slim AS backend-builder
WORKDIR /build

# Instalação unificada de todas as dependências de build detectadas
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config clang libclang-dev swig \
    python3 python3-dev python3-numpy ocl-icd-opencl-dev \
    libclfft-dev libusb-1.0-0-dev git ca-certificates \
    rtl-sdr librtlsdr-dev libopus-dev \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Compilação de Drivers (Padrão Ouro: explicitando diretórios de build)
RUN git clone https://github.com/pothosware/SoapySDR.git /tmp/SoapySDR && \
    cmake -S /tmp/SoapySDR -B /tmp/SoapySDR/build -DCMAKE_BUILD_TYPE=Release && \
    make -C /tmp/SoapySDR/build -j$(nproc) install && \
    git clone https://github.com/pothosware/SoapyRTLSDR.git /tmp/SoapyRTLSDR && \
    cmake -S /tmp/SoapyRTLSDR -B /tmp/SoapyRTLSDR/build -DCMAKE_BUILD_TYPE=Release && \
    make -C /tmp/SoapyRTLSDR/build -j$(nproc) install && \
    ldconfig && rm -rf /tmp/Soapy*

COPY . .
RUN cargo build --release --features "soapysdr,clfft" -p novasdr-server && \
    cargo build --release -p ws_probe

# ESTÁGIO 4: Runtime Final
FROM debian:bookworm-slim
WORKDIR /app

# Adicionado libopus0 e garantido paridade de pacotes
RUN apt-get update && apt-get install -y --no-install-recommends \
    libusb-1.0-0 ocl-icd-libopencl1 libclfft2 rtl-sdr \
    python3 python3-numpy ca-certificates netcat-openbsd \
    libopus0 \
    && rm -rf /var/lib/apt/lists/*

# Cópia completa de bibliotecas e INCLUDES (conforme original)
COPY --from=backend-builder /usr/local/lib/libSoapySDR* /usr/local/lib/
COPY --from=backend-builder /usr/local/lib/SoapySDR /usr/local/lib/SoapySDR
COPY --from=backend-builder /usr/local/include/SoapySDR /usr/local/include/SoapySDR
COPY --from=backend-builder /usr/local/bin/SoapySDRUtil /usr/local/bin/
RUN ldconfig

# Binários e Recursos (Caminhos de origem corrigidos para /build)
COPY --from=backend-builder /build/target/release/novasdr-server /app/
COPY --from=backend-builder /build/target/release/ws_probe /app/
COPY --from=frontend-builder /build/dist /app/frontend/dist
COPY config/ /app/config/
COPY crates/novasdr-server/resources/ /app/resources/

RUN mkdir -p /app/logs /app/data
EXPOSE 9002
ENV RUST_LOG=info RUST_BACKTRACE=1

CMD ["/app/novasdr-server", "-c", "/app/config/config.json", "-r", "/app/config/receivers.json"]
