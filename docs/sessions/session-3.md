# Prompt for Session 3: Cloud Uploading and Resilience (The Broadcaster)

**Goal:** Implement the cloud Uploader Task and the local Monitor UI, focusing on state persistence, S3 resilience, and real-time observability.

**Context & Requirements:**
You will implement Task 3 (Cloud Uploader) and the Axum HTTP Task within the `server` crate.

**1. Task 3: Cloud Uploader (State & Resilience):**
- **Local State Persistence:** Implement logic to read/write `r2_segment` to a local `./recordings/state.json` file via `tokio::fs`. On startup, read the file, extract the `latest` index, and immediately start the task's main loop accepting segments from `last_persisted_index + 1`. If missing, default to 0.
- **Background Cleanup (`tokio::spawn`):** Immediately upon startup, spawn a detached async task to perform a `reqwest` S3 `LIST` on `live/hq/` and `live/lq/`. Delete all keys older than the `last_persisted_index - 10`. Do not block the main Uploader `recv()` loop waiting for this to finish.
- **S3 HTTP Uploads:** Use `reqwest::Client` with `connect_timeout(Duration::from_secs(2))` and `timeout(Duration::from_secs(8))`. Apply the provided `aws_sig_v4` module to sign the requests using `R2_ACCESS_KEY` and `R2_SECRET_KEY`.
- **Atomic Multi-Upload State Machine:**
  1. **Upload HQ (`.flac`):** Retry up to 3 times with exponential backoff (e.g., 500ms, 1s, 2s).
     - Success: Prepend the HQ stream header (`flac_header` from AppState) to the `hq_bytes`, and PUT to S3 at `live/hq/segment-{index}.flac` with `Content-Type: audio/flac`, `Cache-Control: public, max-age=31536000, immutable`.
     - Failure (exhausted): Drop the segment, do NOT update manifest, emit `r2` error SSE, continue to next segment.
  2. **Upload LQ (`.flac`):** Retry up to 3 times.
     - Success: Prepend the LQ stream header (generated once during startup) to `lq_bytes`. PUT to S3 at `live/lq/segment-{index}.flac` with `Content-Type: audio/flac`, `Cache-Control: public, max-age=31536000, immutable`.
     - Failure (exhausted): If HQ succeeded but LQ fails, *abandon the entire index*. Drop the segment, do NOT update the manifest, do NOT push to the rolling window. Emit an `r2` error SSE. (HQ will be cleaned up by lifecycle rules).
  3. **Update Manifest:** If BOTH succeed, PUT `manifest.json` with `{"live": true, "latest": index, "segment_s": 10.24, "qualities": ["hq", "lq"]}`. Use `Content-Type: application/json`, `Cache-Control: no-store, max-age=0`.
- **Rolling Window:** Push the successful keys (`index`) to a `VecDeque`. If `window.len() > 10`, `pop_front()` the oldest index and issue S3 `DELETE` requests for both its HQ and LQ keys. Update `state.json` with the new `latest` index.
- **Local Playback Queue:** Push `(index, hq_bytes)` into `AppState.local_segments` (keep last 3). Ensure the stream header is NOT stored here to save RAM; the HTTP task will prepend it.

**2. HTTP Task & Monitor UI:**
- **Axum Server:** Build an `axum::Router` on `0.0.0.0:8080`.
- **Routes:**
  - `GET /`: Serve `monitor.html` using `Html(include_str!("monitor.html"))`.
  - `GET /events`: Use `axum::response::sse::Event`. Subscribe to `AppState.sse_tx` and yield mapped JSON strings. Include an `Event::default().event("ping").data("keepalive")` every 5 seconds.
  - `GET /local/:id`: Find the index in `local_segments`. Prepend `AppState.flac_header.clone().unwrap()`. Return `audio/flac`.
  - `POST /start`, `POST /stop`: Toggle `AppState.streaming` via atomic memory orderings.
  - `GET /metrics`: Return a Prometheus-formatted string interpolating `radio_capture_overruns_total`, `radio_s3_put_latency_seconds`, etc.

## 3. Testing Contract
- **AWS SigV4 Hashing:** Write a `cargo test` using the exact test vectors provided in the official AWS Signature Version 4 test suite documentation. Your `aws_sig_v4.rs` must produce the exact same `CanonicalRequest` hash and `Signature` string for the given inputs and keys.
- **Manifest Fallback:** Write a test that simulates reading a corrupted `state.json` file. Assert that the server correctly defaults to `index = 0` and attempts the background `LIST` sweep rather than panicking.

## 4. Error Recovery Matrix
- **R2 503 / Rate Limit:** If S3 returns `503 Slow Down`, the exponential backoff loop must naturally sleep and retry up to the 3-attempt limit.
- **R2 403 Forbidden (NTP Drift):** If `reqwest` returns a `403` and the response XML contains `RequestTimeTooSkewed`, log a specific `FATAL: NTP Clock drift detected` message.
- **Split-Brain Upload (HQ Success, LQ Fail):** As defined in the state machine, if LQ exhausts retries after HQ succeeds, you MUST skip the `manifest.json` upload entirely to prevent clients from fetching a 404ing LQ segment. Emit the `r2` error SSE.
