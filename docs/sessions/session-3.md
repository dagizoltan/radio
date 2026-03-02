# Prompt for Session 3: Cloud Uploading and Resilience (The Broadcaster)

**Goal:** Implement the cloud Uploader Task and the local Monitor UI, focusing on state persistence, S3 resilience, and real-time observability.

**Context & Requirements:**
You will implement Task 3 (Cloud Uploader) and the Axum HTTP Task within the `server` crate.

**1. Task 3: Cloud Uploader (State & Resilience):**
- **Local State Persistence:** Implement logic to read/write `r2_segment` to a local `./recordings/state.json` file. On startup, resume from `last_persisted_index + 1`.
- **Background Cleanup:** Spawn a separate background task during startup that performs an S3 `LIST` on `live/hq/` and `live/lq/` and `DELETE`s any orphaned segments older than the rolling window to unblock the main upload pipeline.
- **S3 HTTP Uploads:** Use `reqwest` to perform direct HTTP `PUT` requests to S3/R2. Configure `reqwest` with aggressive timeouts (`connect_timeout`: 2s, `timeout`: 8s). Use the provided AWS Signature V4 logic.
- **Atomicity & Retries:** Implement an exponential backoff retry loop (max 3-5 retries). Upload HQ first, then LQ. **Crucially:** Only advance the manifest `latest` pointer and update the rolling window queue if *both* HQ and LQ uploads succeed. If LQ exhausts retries after HQ succeeds, skip the segment entirely.
- **Cache-Control Headers:**
  - Audio segments (`.flac`, `.opus`): `Cache-Control: public, max-age=31536000, immutable`. Content-Type must be `audio/flac` and `application/octet-stream`.
  - Manifest (`manifest.json`): `Cache-Control: no-store, max-age=0`. Content-Type `application/json`.
- **Rolling Window:** Maintain an in-memory `VecDeque`. When size > 10, actively issue S3 `DELETE`s for the oldest keys.
- **Local Playback:** Prepend the cached stream header to the HQ segment and store it in `local_segments` (keep last 3) for the monitor UI.

**2. HTTP Task & Monitor UI:**
- **Axum Server:** Set up an `axum` server listening on `127.0.0.1:8080`.
- **Routes:**
  - `GET /`: Serve the embedded `monitor.html` UI (`include_str!`).
  - `GET /events`: A Server-Sent Events (SSE) endpoint streaming system state (VU meters, r2 upload status, lag) fed by the pipeline's `broadcast::Sender`.
  - `GET /local/:id`: Serve the complete `.flac` segment from `local_segments`, prepending `AppState.flac_header`.
  - `POST /start`, `POST /stop`: Toggle `AppState.streaming`.
  - `GET /metrics`: Expose Prometheus metrics (overruns, PUT latency, window size).

**Validation:**
Run the server locally against MinIO. Simulate network drops (e.g. `docker pause minio`). Verify that the Uploader gracefully retries, correctly skips indices if retries exhaust without locking up the Recorder, and instantly resumes from `state.json` on a server restart. Ensure the Monitor UI accurately displays VU levels and S3 status via SSE.
