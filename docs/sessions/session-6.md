# Prompt for Session 6: Observability and Docker Polish (Production Readiness)

**Goal:** Finalize the 100% pure-Rust deployment infrastructure, ensure all telemetry accurately reflects the system state, and polish the local operator experience.

## 1. Docker Architecture

Because we removed the C-dependencies (libopus, libasound2), the Dockerfiles should be extremely minimal and fast.

### 1.1 Server Build (`radio-server/Dockerfile`)
Create a multi-stage Dockerfile that builds the Rust server and proves it is pure.

```dockerfile
# Stage 1: Builder
FROM rust:1.77-slim AS builder
WORKDIR /app
COPY . .
# No apt-get install needed! Pure Rust.
RUN cargo build --release

# Validation Step: Prove no dynamic C libraries are linked (except standard libc/libm)
RUN ldd target/release/radio-server
RUN ! ldd target/release/radio-server | grep -E "asound|opus"

# Stage 2: Runner
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/radio-server .
CMD ["./radio-server"]
```

### 1.2 Client Build (`radio-client/Dockerfile`)
Create a multi-stage Dockerfile to build the WASM and run the Deno server.

```dockerfile
# Stage 1: WASM Builder
FROM rust:1.77-slim AS wasm-builder
WORKDIR /app
# Install curl for wasm-pack init
RUN apt-get update && apt-get install -y curl
RUN curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
COPY decoder/flac ./decoder/flac
# Compile the unified FLAC decoder. No Emscripten or LLVM needed!
RUN cd decoder/flac && wasm-pack build --target web

# Stage 2: Deno Runner
FROM denoland/deno:2.0.0
WORKDIR /app
COPY . .
# Copy the compiled WASM into the static folder
COPY --from=wasm-builder /app/decoder/flac/pkg/flac_decoder_bg.wasm ./static/
COPY --from=wasm-builder /app/decoder/flac/pkg/flac_decoder.js ./static/
RUN deno cache main.js
CMD ["run", "--allow-net", "--allow-env", "--allow-read", "main.js"]
```

### 1.3 MinIO Setup (`docker-compose.yml`)
The local dev environment requires a temporary `mc` container to configure the `radio-stream` bucket, make it public, and configure CORS.

```yaml
  minio-setup:
    image: quay.io/minio/mc:latest
    depends_on:
      minio:
        condition: service_healthy
    entrypoint: >
      /bin/sh -c "
      mc alias set local http://minio:9000 minioadmin minioadmin123;
      mc mb --ignore-existing local/radio-stream;
      mc anonymous set download local/radio-stream;
      echo '{\"CORSRules\": [{\"AllowedOrigins\": [\"http://localhost:3000\"], \"AllowedMethods\": [\"GET\"], \"AllowedHeaders\": [\"*\"], \"ExposeHeaders\": [\"ETag\"], \"MaxAgeSeconds\": 3600}]}' > /tmp/cors.json;
      mc anonymous set-json /tmp/cors.json local/radio-stream;
      exit 0;
      "
```

### 1.4 Device Passthrough
In the `radio` service definition in `docker-compose.yml`:
```yaml
  radio:
    build:
      context: ./radio-server
    devices:
      - "/dev/snd:/dev/snd"
    group_add:
      - "audio"
    volumes:
      - "./recordings:/app/recordings"
```

## 2. Observability Baseline (`server.md` & `monitor.html`)

### 2.1 Prometheus Metrics (`src/http.rs`)
The `GET /metrics` endpoint must output a strictly formatted Prometheus text string. Interpolate the `AtomicU64` and `AtomicI32` values directly:

```text
# HELP radio_capture_overruns_total Total ALSA buffer overruns since start
# TYPE radio_capture_overruns_total counter
radio_capture_overruns_total {val}

# HELP radio_segment_upload_exhaustion_total Total segments skipped due to S3 errors
# TYPE radio_segment_upload_exhaustion_total counter
radio_segment_upload_exhaustion_total {val}

# HELP radio_s3_put_latency_seconds Last S3 PUT latency
# TYPE radio_s3_put_latency_seconds gauge
radio_s3_put_latency_seconds{quality="hq"} {val}
radio_s3_put_latency_seconds{quality="lq"} {val}

# HELP radio_rolling_window_size Current number of segments on R2
# TYPE radio_rolling_window_size gauge
radio_rolling_window_size {val}
```

### 2.2 Operator UI Polish (`static/monitor.html`)
The embedded HTML monitor (served by the Axum task) must visually represent the system state:
1. **EventSource Connection:** Open `new EventSource("/events")`.
2. **VU Meters:** Listen for `{"type": "vu", "l": val, "r": val}` events. Map the absolute peak values `0..8388607` (for 24-bit audio) to a CSS `height` or `width` percentage for two `<div>` bars. Color them green, turning red above `-3dBFS` (approx `5931641`).
3. **Queue Sizes:** Display the current index and the size of the rolling window.
4. **Error States:** Listen for `{"type": "r2", "error": true}`. If received, highlight the R2 status block in red and log the timestamp.

## Validation
Run `docker compose up --build`. The system should stand up automatically. Check the build logs to verify the `! ldd ... | grep asound` step passed successfully, proving the C-dependency removal worked. Access `http://localhost:8080` to view the live VU meters reacting to the ALSA stream, and confirm metrics are actively exported at `/metrics`.
