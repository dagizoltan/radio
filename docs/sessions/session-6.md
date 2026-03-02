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
- **Client Build (`radio-client/Dockerfile`):** Write a multi-stage Dockerfile. Stage 1 (Builder): Use `rust:1.77-bullseye`. Install `curl`, `build-essential`. Install `wasm-pack`. Install `llvm` and `clang` to provide `wasi-sdk` cross-compilation tools for the `libopus` C dependency. Run:
  `CC=clang AR=llvm-ar wasm-pack build --target web decoder/opus/`
  Stage 2 (Runner): Use `denoland/deno:2.0.0`. Copy the `.wasm` and `.js` outputs from the builder into the `static/` dir. Run `deno run --allow-net --allow-env --allow-read main.js`.
- **Server Build (`radio-server/Dockerfile`):** Stage 1: `rust:1.77-slim`. `apt install pkg-config libopus-dev libasound2-dev`. Compile the binary. Add a `RUN ldd target/release/radio-server | grep opus` step. Stage 2: `debian:bookworm-slim`. `apt install libopus0 libasound2`. Copy binary.
- **MinIO Setup:** Write a script for the `minio-setup` container that creates `radio-stream`, runs `mc anonymous set download minio/radio-stream`, and sets a JSON CORS policy allowing `GET` from `http://localhost:3000` with `ExposeHeaders: ["ETag"]`.
- **Device Passthrough:** In `docker-compose.yml`, mount `/dev/snd:/dev/snd` to the `radio` service. Use `group_add: ["audio"]` to ensure permission maps correctly.

**Validation:**
Run `docker compose up --build`. The system should stand up automatically. Verify the WASM decoders compile without C linking errors. Access `http://localhost:8080` to view the live VU meters and confirm metrics are exported at `/metrics`. Force a CPU spike or pull the USB cable to trigger an `EPIPE` and verify the `radio_capture_overruns_total` metric increments.
