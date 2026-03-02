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
     - Success: Prepend the stream header (`flac_header` from AppState) to the `hq_bytes`, and PUT to S3 with `Content-Type: audio/flac`, `Cache-Control: public, max-age=31536000, immutable`.
     - Failure (exhausted): Drop the segment, do NOT update manifest, emit `r2` error SSE, continue to next segment.
  2. **Upload LQ (`.opus`):** Retry up to 3 times.
     - Success: PUT `lq_bytes` to S3 with `Content-Type: application/octet-stream`, `Cache-Control: public, max-age=31536000, immutable`.
     - Failure (exhausted): If HQ succeeded but LQ fails, *abandon the entire index*. Drop the segment, do NOT update the manifest, do NOT push to the rolling window. Emit an `r2` error SSE. (HQ will be cleaned up by lifecycle rules).
  3. **Update Manifest:** If BOTH succeed, PUT `manifest.json` with `{"live": true, "latest": index, "segment_s": 10.24, ...}`. Use `Content-Type: application/json`, `Cache-Control: no-store, max-age=0`.
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

**Validation:**
Run the server locally against MinIO. Simulate network drops (e.g. `docker pause minio`). Verify that the Uploader gracefully retries, correctly skips indices if retries exhaust without locking up the Recorder, and instantly resumes from `state.json` on a server restart. Ensure the Monitor UI accurately displays VU levels and S3 status via SSE.
