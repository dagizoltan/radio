# Prompt for Session 6: Observability and Docker Polish (Production Readiness)

**Goal:** Finalize the deployment infrastructure, ensure all telemetry accurately reflects the system state, and polish the local operator experience.

**Context & Requirements:**
This session is focused on the `docker-compose.yml`, Dockerfiles, and the observability baseline.

**1. Observability Baseline (`server.md` & `monitor.html`):**
- **Prometheus Metrics:** Ensure the `GET /metrics` endpoint accurately outputs `radio_capture_overruns_total` (driven by EPIPE recoveries), `radio_segment_upload_exhaustion_total` (driven by S3 backoff failures), S3 PUT latencies, and the rolling window size.
- **Operator UI Polish:** Ensure the embedded HTML monitor visually updates the left/right VU meters dynamically based on the SSE event stream, clearly displays the `live/hq` and `live/lq` queue sizes, and surfaces any R2 `error` states prominently.

**2. Docker Architecture:**
- **Client Build Requirements:** Ensure the `radio-client/Dockerfile` correctly installs the Rust `wasm32-unknown-unknown` target, `wasm-pack`, and crucially, Emscripten (`emcc`) or a `wasi-sdk` environment so that the C `libopus` dependency in `decoder/opus` compiles to WASM successfully.
- **Server Build Requirements:** Ensure the `radio-server/Dockerfile` installs `pkg-config`, `libopus-dev`, and `libasound2-dev` during the build stage, and `libopus` and `libasound2` in the runtime stage. Add a CI-style validation command to the build process: `ldd target/release/radio-server | grep opus` to prove it linked correctly.
- **MinIO Setup:** Ensure the `minio-setup` container script correctly sets up the `radio-stream` bucket, applies the anonymous download policy, and applies a CORS rule allowing `GET` requests with `ExposeHeaders: ["ETag"]` for the `http://localhost:3000` origin.
- **Device Passthrough:** Ensure the Compose file provides instructions or configuration for mapping `/dev/snd` and matching audio group permissions so the container can natively read the UMC404HD.

**Validation:**
Run `docker compose up --build`. The system should stand up automatically. Verify the WASM decoders compile without C linking errors. Access `http://localhost:8080` to view the live VU meters and confirm metrics are exported at `/metrics`. Force a CPU spike or pull the USB cable to trigger an `EPIPE` and verify the `radio_capture_overruns_total` metric increments.
