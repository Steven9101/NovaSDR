# ==========================================
# ESTÁGIO 1: Frontend (Node.js)
# ==========================================
FROM node:20-slim AS frontend-builder
WORKDIR /build
COPY frontend/package*.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ==========================================
# ESTÁGIO 2: Planejamento de Receita (Rust)
# ==========================================
FROM rustlang/rust:nightly-bookworm-slim AS planner
WORKDIR /app
RUN cargo install cargo-chef
COPY . .
# Gera um arquivo de "receita" com as dependências
RUN cargo chef prepare --recipe-path recipe.json

# ==========================================
# ESTÁGIO 3: Compilação de Dependências e Ferramentas C++
# ==========================================
FROM rustlang/rust:nightly-bookworm-slim AS backend-builder
WORKDIR /app

# Instalação de dependências do sistema
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config clang libclang-dev swig \
    python3 python3-dev python3-numpy ocl-icd-opencl-dev \
    libclfft-dev libusb-1.0-0-dev git ca-certificates \
    rtl-sdr librtlsdr-dev \
    && rm -rf /var/lib/apt/lists/*

# Instala o cargo-chef para cozinhar as dependências
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json

# "Cozinha" as dependências (Camada de Cache pesada)
RUN cargo chef cook --release --recipe-path recipe.json

# Compila SoapySDR e drivers (Não mudam com frequência)
RUN git clone https://github.com/pothosware/SoapySDR.git /tmp/SoapySDR && \
    cmake -S /tmp/SoapySDR -B /tmp/SoapySDR/build -DCMAKE_BUILD_TYPE=Release && \
    make -C /tmp/SoapySDR/build -j$(nproc) install && \
    git clone https://github.com/pothosware/SoapyRTLSDR.git /tmp/SoapyRTLSDR && \
    cmake -S /tmp/SoapyRTLSDR -B /tmp/SoapyRTLSDR/build -DCMAKE_BUILD_TYPE=Release && \
    make -C /tmp/SoapyRTLSDR/build -j$(nproc) install && \
    ldconfig && rm -rf /tmp/Soapy*

# Agora sim, copia o código real e compila os binários
COPY . .
RUN cargo build --release --features "soapysdr,clfft" -p novasdr-server && \
    cargo build --release -p ws_probe

# ==========================================
# ESTÁGIO 4: Imagem de Runtime Final
# ==========================================
FROM debian:bookworm-slim
WORKDIR /app

# Dependências mínimas de execução
RUN apt-get update && apt-get install -y --no-install-recommends \
    libusb-1.0-0 ocl-icd-libopencl1 libclfft2 rtl-sdr \
    python3 python3-numpy ca-certificates netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*

# Copia bibliotecas compiladas do SoapySDR
COPY --from=backend-builder /usr/local/lib/libSoapySDR* /usr/local/lib/
COPY --from=backend-builder /usr/local/lib/SoapySDR /usr/local/lib/SoapySDR
COPY --from=backend-builder /usr/local/bin/SoapySDRUtil /usr/local/bin/
RUN ldconfig

# Copia Binários, Frontend, Configs e Recursos
COPY --from=backend-builder /app/target/release/novasdr-server /app/
COPY --from=backend-builder /app/target/release/ws_probe /app/
COPY --from=frontend-builder /build/dist /app/frontend/dist
COPY config/ /app/config/
COPY crates/novasdr-server/resources/ /app/resources/

# Preparação de diretórios e ambiente
RUN mkdir -p /app/logs /app/data
EXPOSE 9002
ENV RUST_LOG=info RUST_BACKTRACE=1

CMD ["/app/novasdr-server", "-c", "/app/config/config.json", "-r", "/app/config/receivers.json"]
