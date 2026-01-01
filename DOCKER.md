# NovaSDR Docker Deployment Guide

This guide explains how to build and run NovaSDR using Docker and Docker Compose.

## Prerequisites

- Docker Engine 20.10 or later
- Docker Compose 2.x or later
- SDR hardware (RTL-SDR, HackRF, Airspy, etc.) - optional if using stdin mode
- Sufficient USB permissions for SDR device access

## Quick Start

### 1. Build the Docker image

```bash
docker compose build
```

This will create a multi-stage build that:
- Builds the frontend (React/Vite)
- Compiles the Rust backend with SoapySDR and OpenCL clFFT support
- Installs SoapySDR and SoapyRTLSDR from source
- Creates an optimized runtime image

Build time: 10-30 minutes depending on your system.

### 2. Initial Setup

**Important:** Before starting the container for the first time, you need to run the interactive setup wizard to configure your receivers:

```bash
docker compose run --rm novasdr /app/novasdr-server -c /app/config/config.json -r /app/config/receivers.json
```

This will:
- Launch the NovaSDR setup wizard
- Guide you through configuring server settings
- Detect and configure your SDR devices
- Create the required configuration files in `config/config.json` and `config/receivers.json`

You can also manually edit configuration files in the `config/` directory:

- `config/config.json` - Server settings (port, hostname, limits)
- `config/receivers.json` - SDR receiver configuration

### 3. Run

Once configured, start the server:

```bash
docker compose up -d
```

Access the web UI at: `http://localhost:9002`

### 4. View logs

```bash
docker compose logs -f novasdr
```

### 5. Stop

```bash
docker compose down
```

## Configuration

### Server Configuration (`config/config.json`)

Key settings:
- `server.port` - Web server port (default: 9002)
- `server.host` - Bind address (use "0.0.0.0" for Docker)
- `server.html_root` - Frontend path (default: "frontend/dist/")
- `limits.audio` - Maximum audio clients
- `limits.waterfall` - Maximum waterfall clients

### Receiver Configuration (`config/receivers.json`)

Configure your SDR receivers here. Example for RTL-SDR:

```json
{
  "receivers": [
    {
      "name": "RTL-SDR",
      "input": {
        "type": "soapysdr",
        "driver": "rtlsdr",
        "sps": 2048000,
        "frequency": 100900000,
        "gain_mode": "manual",
        "gain": 48.0
      },
      "dsp": {
        "accelerator": "clfft",
        "fft_size": 8192
      }
    }
  ]
}
```

## USB Device Access

### Linux

Grant Docker access to USB devices by:

1. **Using privileged mode** (easiest, less secure):
   ```yaml
   privileged: true
   ```

2. **Mapping specific devices** (recommended):
   ```yaml
   devices:
     - /dev/bus/usb:/dev/bus/usb
   ```

3. **Using device cgroup rules**:
   ```yaml
   device_cgroup_rules:
     - 'c 189:* rmw'  # USB devices
   ```

### Verifying SDR Detection

Check if SoapySDR detects your device:

```bash
docker compose exec novasdr SoapySDRUtil --probe
```

For RTL-SDR specifically:

```bash
docker compose exec novasdr SoapySDRUtil --probe="driver=rtlsdr"
```

## OpenCL/GPU Acceleration

The Docker image includes OpenCL support for GPU-accelerated FFT processing.

### Configure OpenCL Device

Set environment variables in `docker-compose.yml`:

```yaml
environment:
  - NOVASDR_OPENCL_PLATFORM=0
  - NOVASDR_OPENCL_DEVICE=0
```

### Verify OpenCL

```bash
docker compose exec novasdr clinfo
```

### GPU Access

For NVIDIA GPUs with CUDA:

```yaml
services:
  novasdr:
    runtime: nvidia
    environment:
      - NVIDIA_VISIBLE_DEVICES=all
```

For AMD GPUs, ensure the container has access to `/dev/dri`.

## Alternative Builds

### CPU-Only Build (No OpenCL)

Modify the Dockerfile build command:

```dockerfile
RUN cargo build --release --features "soapysdr" -p novasdr-server
```

### With VkFFT (Vulkan)

For Vulkan-based acceleration (Raspberry Pi 4, etc.):

```dockerfile
RUN apt-get install -y libvkfft-dev libvulkan-dev glslang-dev spirv-tools g++
RUN cargo build --release --features "soapysdr,vkfft" -p novasdr-server
```

## stdin Mode (Pipe from External Tool)

If you want to pipe samples from an external SDR tool instead of using SoapySDR:

```yaml
services:
  novasdr:
    # ... other config ...
    command: ["/bin/sh", "-c", "rtl_sdr -g 48 -f 100900000 -s 2048000 - | /app/novasdr-server -c /app/config/config.json -r /app/config/receivers.json"]
```

## Reverse Proxy Setup

### Nginx

```nginx
location / {
    proxy_pass http://localhost:9002;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

### Traefik

```yaml
labels:
  - "traefik.enable=true"
  - "traefik.http.routers.novasdr.rule=Host(`sdr.example.com`)"
  - "traefik.http.services.novasdr.loadbalancer.server.port=9002"
```

## Troubleshooting

### Container won't start

Check logs:
```bash
docker compose logs novasdr
```

### SDR device not detected

1. Verify USB permissions
2. Check device is not in use by another process
3. Try privileged mode
4. Verify udev rules on host system

### Frontend not loading

1. Ensure frontend was built correctly
2. Check `html_root` path in config.json
3. Verify port 9002 is accessible

### WebSocket connection issues

1. Check firewall rules
2. Verify reverse proxy configuration (if using one)
3. Check browser console for errors

### Out of memory

Increase Docker memory limits:
```yaml
deploy:
  resources:
    limits:
      memory: 4G
```

## Production Deployment

### Security Considerations

1. **Don't run as privileged** unless absolutely necessary
2. **Use specific device mappings** instead of full USB access
3. **Set resource limits** to prevent resource exhaustion
4. **Use a reverse proxy** with HTTPS/TLS
5. **Restrict network access** using firewall rules

### Performance Optimization

1. **CPU pinning**: Pin container to specific CPU cores
2. **Memory limits**: Set appropriate memory limits based on workload
3. **GPU acceleration**: Use clFFT or VkFFT for better performance
4. **Network**: Use host network mode for better latency

### Monitoring

Use Docker health checks:

```bash
docker compose ps
```

View resource usage:

```bash
docker stats novasdr-server
```

## Building for Different Architectures

### ARM64 (Raspberry Pi 4, etc.)

```bash
docker buildx build --platform linux/arm64 -t novasdr:arm64 .
```

### Multi-platform

```bash
docker buildx build --platform linux/amd64,linux/arm64 -t novasdr:latest .
```

## Updates

To update to a newer version:

```bash
git pull
docker compose build --no-cache
docker compose up -d
```

## Support

For issues and questions:
- GitHub Issues: https://github.com/Steven9101/NovaSDR/issues
- Documentation: See `docs/` directory
- Operations Guide: `docs/OPERATIONS.md`

## License

NovaSDR is licensed under GPL-3.0-only. See LICENSE file for details.
