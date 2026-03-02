# Prompt for Session 3: Cloud Uploading and Resilience (The Broadcaster)

**Goal:** Implement the cloud Uploader Task and the local Monitor UI, focusing on state persistence, S3 resilience via precise SigV4 signing, and real-time axum observability.

## 1. Cloud Uploader Initialization & State (`src/uploader.rs`)

### 1.1 Local State Persistence
1. On task start, attempt to read `./recordings/state.json`.
   ```json
   { "latest_index": 42 }
   ```
2. If the file exists, parse it and set `current_index = latest_index + 1`. If it doesn't exist, set `current_index = 0`.
3. Set `state.r2_segment.store(current_index, Ordering::SeqCst)`.
4. Define a helper `async fn save_state(index: u64)` that writes this JSON back to disk atomically (write to `state.json.tmp` and `tokio::fs::rename`).

### 1.2 Background Cleanup Sweep
Immediately upon starting, spawn a detached background task to prevent blocking the upload pipeline with slow paginated `LIST` calls.
```rust
tokio::spawn(async move {
    // 1. Perform S3 LIST on `live/hq/` and `live/lq/`
    // 2. Identify all segment keys where the parsed integer index is less than `current_index - 10`.
    // 3. Issue S3 DELETE requests for these orphaned keys.
});
```

### 1.3 reqwest Client Setup
Initialize the HTTP client to fail fast and prevent channel deadlock.
```rust
let client = reqwest::Client::builder()
    .connect_timeout(Duration::from_secs(2))
    .timeout(Duration::from_secs(8))
    .build()?;
```

## 2. AWS Signature V4 Implementation (`src/aws_sig_v4.rs`)
Implement the signing logic manually to avoid heavy/unstable SDK crates.
1. **Payload Hash:** Calculate `SHA256(payload)`. For `manifest.json`, hash the string. For audio bytes, hash the bytes. For empty payloads (e.g. DELETE), hash the empty string.
2. **Canonical Request:** Construct exactly:
   ```text
   <HTTPMethod>\n<CanonicalURI>\n<CanonicalQueryString>\n<CanonicalHeaders>\n<SignedHeaders>\n<HashedPayload>
   ```
3. **String to Sign:** Construct:
   ```text
   AWS4-HMAC-SHA256\n<Timestamp(YYYYMMDD'T'HHMMSS'Z')>\n<YYYYMMDD>/auto/s3/aws4_request\n<SHA256(CanonicalRequest)>
   ```
4. **Signing Key:** Derive using HMAC-SHA256:
   `HMAC(HMAC(HMAC(HMAC("AWS4" + SecretKey, "YYYYMMDD"), "auto"), "s3"), "aws4_request")`
5. **Signature:** `HMAC(SigningKey, StringToSign)`
6. **Authorization Header:**
   `AWS4-HMAC-SHA256 Credential=<AccessKey>/<YYYYMMDD>/auto/s3/aws4_request, SignedHeaders=<headers>, Signature=<Signature>`

## 3. Atomic Multi-Upload State Machine (`src/uploader.rs`)

Inside the `while let Some((index, mut hq_bytes, mut lq_bytes)) = seg_rx.recv().await` loop:

1. **Prepend Headers:**
   - Lock `state.flac_header`, clone the bytes.
   - Prepend this header to `hq_bytes`.
   - The Converter generated a specific 24kHz header for the LQ stream on startup. Ensure that specific LQ header is prepended to `lq_bytes`.

2. **Upload HQ (`.flac`):**
   - Execute a retry loop (max 3 tries).
   - If `reqwest::Error` occurs, `tokio::time::sleep(Duration::from_millis(500 * 2^attempt))`.
   - PUT URL: `{R2_ENDPOINT}/{R2_BUCKET}/live/hq/segment-{index:08}.flac`
   - Headers: `Content-Type: audio/flac`, `Cache-Control: public, max-age=31536000, immutable`.
   - If all retries exhaust: Log `ERROR`, emit `{"type": "r2", "error": true}` to `sse_tx`, `continue` to the next segment in the main `recv()` loop. Do NOT advance manifest.

3. **Upload LQ (`.flac`):**
   - Execute the same retry loop for LQ.
   - PUT URL: `{R2_ENDPOINT}/{R2_BUCKET}/live/lq/segment-{index:08}.flac`
   - Headers: `Content-Type: audio/flac`, `Cache-Control: ...`
   - If all retries exhaust: Log `ERROR`. Because HQ succeeded but LQ failed, this segment index is "split-brain". Treat it as a total failure. Emit `error` SSE and `continue` to the next segment. (The orphaned HQ segment will be cleaned up by the background task on the next restart or by bucket lifecycle rules).

4. **Commit Phase (Manifest & Rolling Window):**
   - Only reached if BOTH uploads succeeded.
   - Construct `manifest.json`:
     `{"live": true, "latest": index, "segment_s": 10.24, "updated_at": <unix_ms>, "qualities": ["hq", "lq"]}`
   - PUT `manifest.json` with `Cache-Control: no-store, max-age=0`.
   - On manifest success:
     - Push `index` to a local `VecDeque`. If `len > 10`, `pop_front()` the oldest index `X`.
     - Spawn a fast detached task to issue `DELETE /live/hq/segment-{X:08}.flac` and `DELETE /live/lq/segment-{X:08}.flac`.
     - Await `save_state(index)`.
     - Update `state.r2_last_ms` and `state.r2_segment`. Emit `{"type": "r2", "uploading": false}` to `sse_tx`.
     - Push `(index, hq_bytes)` into `state.local_segments`. `pop_front()` if `len > 3`.

## 4. HTTP Task & Monitor UI (`src/http.rs`)

Set up `axum` on `0.0.0.0:8080`.

- `GET /`: Return `axum::response::Html(include_str!("../static/monitor.html"))`.
- `GET /events`:
  ```rust
  let mut rx = state.sse_tx.subscribe();
  let stream = async_stream::stream! {
      while let Ok(msg) = rx.recv().await {
          yield Ok(axum::response::sse::Event::default().data(msg));
      }
  };
  Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
  ```
- `GET /local/:id`: Parse the 8-digit index string. Lock `state.local_segments`. Find the matching index. Lock `state.flac_header`. Concatenate header + bytes and return as `audio/flac`.
- `GET /metrics`: Return a hardcoded string interpolating the state atomics for Prometheus scraping:
  ```text
  # HELP radio_capture_overruns_total Total ALSA buffer overruns
  # TYPE radio_capture_overruns_total counter
  radio_capture_overruns_total {val}
  ```

## Validation
Set up a mock local HTTP server (or local MinIO container) and configure the environment variables. Ensure the AWS SigV4 implementation successfully authenticates. Force the network interface down momentarily and verify the Uploader Task pauses, retries, recovers, and correctly drops the segment if the timeout is exceeded without panicking the entire application.