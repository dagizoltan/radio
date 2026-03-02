# Prompt for Session 6: Observability and Docker Polish (Production Readiness)

**Goal:** Finalize the deployment infrastructure, ensure all telemetry accurately reflects the system state, and polish the local operator experience.

**Context & Requirements:**
This session is focused on the `docker-compose.yml`, Dockerfiles, and the observability baseline.

**1. Observability Baseline (`server.md` & `monitor.html`):**
- **Prometheus Metrics:** Format the `/metrics` endpoint string output exactly as:
  ```text
  # HELP radio_capture_overruns_total Total ALSA buffer overruns since start
  # TYPE radio_capture_overruns_total counter
  radio_capture_overruns_total {val}

  # HELP radio_segment_upload_exhaustion_total Total segments skipped
  # TYPE radio_segment_upload_exhaustion_total counter
  radio_segment_upload_exhaustion_total {val}

  # HELP radio_s3_put_latency_seconds Last S3 PUT latency
  # TYPE radio_s3_put_latency_seconds gauge
  radio_s3_put_latency_seconds{quality="hq"} {val}
  ```
- **Operator UI Polish:** Ensure the embedded HTML monitor visually updates the left/right VU meters dynamically based on the SSE event stream, clearly displays the `live/hq` and `live/lq` queue sizes, and surfaces any R2 `error` states prominently.

**2. Docker Architecture:**
- **Client Build (`radio-client/Dockerfile`):** Write a multi-stage Dockerfile. Stage 1 (Builder): Use `rust:1.77-bullseye`. Install `wasm-pack`. (No C dependencies needed). Run:
  `wasm-pack build --target web decoder/flac/`
  Stage 2 (Runner): Use `denoland/deno:2.0.0`. Copy the `.wasm` and `.js` outputs from the builder into the `static/` dir. Run `deno run --allow-net --allow-env --allow-read main.js`.
- **Server Build (`radio-server/Dockerfile`):** Stage 1: `rust:1.77-slim`. (No C dependencies needed). Compile the binary `cargo build --release`. Add a `RUN ldd target/release/radio-server` step to prove it dynamically links only standard system libraries. Stage 2: `debian:bookworm-slim`. Copy binary.
- **MinIO Setup:** Write a script for the `minio-setup` container that creates `radio-stream`, runs `mc anonymous set download minio/radio-stream`, and sets a JSON CORS policy allowing `GET` from `http://localhost:3000` with `ExposeHeaders: ["ETag"]`.
- **Device Passthrough:** In `docker-compose.yml`, mount `/dev/snd:/dev/snd` to the `radio` service. Use `group_add: ["audio"]` to ensure permission maps correctly.

## 3. Testing Contract
- **C-Dependency Verification:** Write a shell script assertion in the server's Dockerfile: `RUN ! ldd target/release/radio-server | grep -E "asound|opus"`. If the binary contains links to C audio libraries, the Docker build MUST fail.
- **CORS Assertion:** Perform a `curl -I -H "Origin: http://localhost:3000" http://localhost:9000/radio-stream/live/manifest.json`. Assert that `Access-Control-Allow-Origin: *` or matching origin is present in the response headers.

## 4. Error Recovery Matrix
- **MinIO Startup Race:** The `radio` and `client` containers will start before `minio` is fully ready. Use `depends_on: minio: condition: service_healthy` in `docker-compose.yml` to prevent crash-looping during local development.
- **ALSA Device Missing:** If `/dev/snd` is not mapped correctly by Docker, the server will panic on startup. Ensure the `README` clearly documents the `--device` flag and `group_add` requirements for Linux hosts.
