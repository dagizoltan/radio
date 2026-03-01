# Server Crate

The `crates/server` library is the main binary that orchestrates the entire `radio-server` architecture. It initializes the Tokio runtime and runs four concurrent tasks.

## Shared State

All state is held in an `Arc<AppState>` and shared safely across tasks:

*   `streaming: AtomicBool`: Toggled by the `/start` and `/stop` HTTP endpoints. When `false`, the Recorder Task must pause writing to the local archive and pause sending PCM data to the Converter Task, effectively stopping the live stream output and conserving disk/CPU resources without tearing down the ALSA handle.
*   `vu_left, vu_right: AtomicI32`: Peak sample values continuously updated by the pipeline for the UI VU meters.
*   `recording_path: Mutex<String>`: The full path to the current local archive file (e.g., `./recordings/recording-1234.flac`).
*   `recording_bytes: AtomicU64`: The total number of bytes written to the current archive file.
*   `r2_segment: AtomicU64`: The monotonic index of the current active segment.
*   `r2_last_ms: AtomicU64`: Unix timestamp in milliseconds of the last successful segment upload.
*   `r2_uploading: AtomicBool`: True when the R2 uploader task is actively performing an HTTP PUT.
*   `local_segments: Mutex<VecDeque<(u64, Bytes)>>`: A rolling window containing the last 3 segments in RAM. Used for the local `/local/:id` playback route.
*   `flac_header: Mutex<Option<Bytes>>`: The cached FLAC stream header (`fLaC` + `STREAMINFO`). Prepended to local segments before serving them.
*   `sse_tx: broadcast::Sender<String>`: The event bus for broadcasting state changes to connected Server-Sent Events (SSE) clients on the monitor UI.

## Process Architecture (Tokio Tasks)

The system is logically divided into three primary processes handling audio capture, multi-quality conversion, and cloud upload. These, along with an HTTP task, run concurrently in `main` using `tokio::select!`.

### Process 1: HQ Recorder Task

Handles direct hardware capture and local uncompressed archiving.

1.  Opens the ALSA capture device using the [Capture Crate](capture.md).
2.  Initializes a high-quality (HQ) `FlacEncoder` instance for raw encoding.
3.  Creates a new, timestamped staging file in a fast, local container directory (e.g., `/tmp/` or a `tmpfs` mount) to prevent the capture loop from blocking on slow host-mounted filesystems (especially relevant if a developer runs this on Docker Desktop for Mac/Windows).
4.  Enters an asynchronous loop, awaiting periods from the capture device.
5.  **For each period (4096 frames):**
    *   Computes the peak absolute sample value for the left and right channels for the UI. Updates `vu_left` and `vu_right`.
    *   Encodes the raw `&[i32]` buffer into raw verbatim HQ 24-bit FLAC frames.
    *   Writes the frames directly to the staging archive file using `AsyncWriteExt::write_all`. Updates `recording_bytes`.
    *   Sends the raw PCM period as `Arc<Vec<i32>>` via a bounded `tokio::sync::mpsc` channel (capacity 16) to the Converter Task. The FLAC encoding above is written to the archive only — the Converter receives raw PCM directly and never sees the FLAC-encoded bytes.
    *   Emits VU levels to `sse_tx`.
6.  **Archiving:** Periodically (e.g., every 60 minutes) or on graceful shutdown, it flushes the current staging file, closes the handle, and asynchronously moves it to the host-mounted `./recordings/` directory for long-term storage, opening a new staging file to continue.

### Process 2: Converter Task

Consumes the raw stream, normalizes it, and encodes it into multiple qualities (HQ and LQ).

1.  Receives from the bounded `mpsc` channel. The `broadcast` channel carries only JSON event strings for the monitor UI SSE feed. Shutdown is coordinated via a cancellation token, not via the broadcast channel.
2.  Initializes the `Normalizer`.
3.  Initializes the audio encoders: one `FlacEncoder` for the normalized HQ stream (24-bit), and one Opus encoder (`audiopus::Encoder` configured for `48000 Hz`, `Stereo`, `Application::Audio`, bitrate `128000 bps`) with an `ogg::PacketWriter` for container framing. Before encoding, samples are converted from `i32` to `f32` (range `[-1.0, 1.0]`). The Opus encoder produces variable-length packets; the Ogg writer accumulates them into pages. Each 10-second segment is assembled as a complete, self-contained Ogg Opus stream (includes the two header pages prepended).
4.  Loops, receiving raw PCM periods from the Recorder Task via the bounded `mpsc` channel. Each message is an `Arc<Vec<i32>>` containing one period of 4096 interleaved frames. No decoding step is required — the PCM arrives ready for normalisation.
5.  Copies the raw buffer to a new mutable buffer and calls `normalizer.process(&mut buffer)`.
6.  Encodes the normalized buffer simultaneously into the respective HQ (FLAC) and LQ (Opus) streams.
7.  Emits the current normalizer gain to `sse_tx` every 50ms using a `tokio::time::interval`.
8.  **Segment Assembly:** Accumulates the encoded frames for both qualities.
    *   *Optimization Strategy:* To prevent constant vector reallocations as frames accumulate, the accumulator `Vec<u8>` for the 10-second segment must be pre-allocated with `Vec::with_capacity()`. A 10-second 24-bit verbatim FLAC segment contains the raw PCM payload (`48000 * 3 bytes * 2 channels * 10s = 2,880,000` bytes) plus the FLAC framing overhead. A 10s segment has ~117 FLAC frames, each contributing a frame header (~10-15 bytes) and a CRC-16 (2 bytes), adding around ~1,700 bytes. To ensure zero reallocations, the buffer must be padded slightly larger than just the PCM size.
    *   **Threshold:** 480,000 frames (10 × 48000 Hz). Track a `frame_counter: u64` incremented by `period_frames` (4096) per ALSA period. Pre-allocate accumulator: `Vec::with_capacity(2_885_000)` for HQ (PCM + framing overhead padding).
    *   Assembles complete, standalone files in memory by prepending the respective stream headers to the filled accumulator.
    *   Sends completed HQ and LQ segment files to the Cloud Uploader via a bounded `tokio::sync::mpsc` channel with capacity **3**. If the channel is full (Uploader lagging), the Converter logs `WARN: uploader lagging, oldest segment dropped` and the channel's back-pressure causes the send to fail gracefully.

### Process 3: Cloud Uploader Task

Receives completed segments and handles S3 uploads and manifest management.

1.  **Startup Cleanup:** Before processing any new segments, performs a `LIST` and `DELETE` operation on the bucket prefixes (`live/hq/` and `live/lq/`) to remove any orphaned segments from a previous crashed session.
    *   **Race Condition Mitigation:** Before issuing any `DELETE` requests during the startup sweep, write `manifest.json` with `"live": false`. This ensures any active listeners enter their backoff/retry state before their buffered segments are deleted. After the sweep completes and the first new segment is uploaded, the manifest is updated to `"live": true`. The recommended implementation sequence:
        1. LIST `live/hq/` and `live/lq/` to find all existing segment keys and the highest existing index.
        2. Write `manifest.json` with `"live": false` and `latest` set to the highest found index (or `0` if none found). Example: `{"live": false, "latest": 99, "segment_s": 10, "updated_at": <now>}`. Using the previously-known index prevents active clients from hard-resetting to segment 0 when they read the offline manifest.
        3. DELETE all found segment keys.
        4. Resume uploading from `(highest_found_index + 1) % 100_000_000` (or `0` if none found).
        5. After the first successful segment upload: write `manifest.json` with `"live": true` and the new `latest` index.

> **Why LIST before writing the offline manifest:** Writing `"latest": 0` in the offline manifest causes all connected clients to reset their `currentIndex` to 0. When `live: true` is published with the real index (e.g., 1,042), every client triggers the jump-ahead logic and makes a burst of manifest polls simultaneously. Using the last known index avoids this thundering-herd effect on reconnect.

2.  Receives completed HQ and LQ segment files from the Converter Task via a bounded `tokio::sync::mpsc` channel (capacity 3). The Uploader is the sole receiver. If the channel is full due to upload back-pressure, the Converter's send returns an error and logs `WARN: uploader lagging`.
3.  Loops, receiving assembled segment files.
4.  **Upload:**
    *   Uploads the segments to S3 using raw HTTP with [AWS Signature V4](aws-sig-v4.md).
    *   **Resilience:** Uses an exponential backoff retry loop (e.g., 3-5 retries) for the HTTP PUT requests to handle transient network drops.
    *   Key format uses quality folders: e.g., `live/hq/segment-{:08}.flac` and `live/lq/segment-{:08}.opus`.
    *   Writes `live/manifest.json` containing metadata for both streams: `{"live": true, "latest": index, "segment_s": 10, "updated_at": timestamp, "qualities": ["hq", "lq"]}`.
    *   **Crucial Caching Instruction:** The `PUT` request for `manifest.json` **must** include the `Cache-Control: no-cache, no-store, must-revalidate` metadata explicitly. This prevents aggressive edge caching by Cloudflare R2 (or an upstream CDN), ensuring the browser receives `304 Not Modified` *only* via the backend ETag mechanism, preventing the stream from "ghosting" or appearing offline while still actively uploading.
5.  **Rolling Window:** Maintains a queue of uploaded S3 keys for all qualities. If the window exceeds 3 segments per quality, it issues `DELETE` requests for the oldest objects.
6.  Pushes the new HQ segment into `local_segments` (keeping only the last 3) for the local operator monitor playback.
7.  Updates `r2_segment`, `r2_last_ms`, and `r2_uploading`. Emits `r2` status events to `sse_tx` during and after the upload.

### HTTP Task

Serves the local monitor UI on `127.0.0.1:8080` using `axum`.

*   `GET /`: Serves `monitor.html` embedded via `include_str!()`.
*   `GET /events`: Subscribes to `sse_tx` and streams Server-Sent Events. Includes a 5-second keepalive ping.
*   `POST /start`: Sets `streaming: true`, emits a status event.
*   `POST /stop`: Sets `streaming: false`, emits a status event.
*   `GET /status`: Returns a JSON snapshot of the entire `AppState`.
*   `GET /local/:id`: Looks up the segment by index in `local_segments`. Prepends the `flac_header` and returns the complete bytes as `audio/flac` with `Cache-Control: no-cache`.

## Critical Constraints

**CRITICAL CONSTRAINT:** Two dedicated audio `mpsc` channels, not broadcast. Raw PCM flows Recorder→Converter via `mpsc(16)`. Assembled segments flow Converter→Uploader via `mpsc(3)`. The `broadcast` channel carries only SSE event bus messages. The local archive must be completely unprocessed — the normalizer must never touch the recorded audio.

**CRITICAL CONSTRAINT:** Rolling window, not TTL. R2 has no native TTL. The uploader maintains a VecDeque of uploaded keys and deletes the oldest immediately when the window exceeds 3 segments. At any moment R2 holds exactly 3 segments and one manifest.

**CRITICAL CONSTRAINT:** Segments are complete FLAC files. Every segment uploaded to R2 must be playable as a standalone FLAC file.
### Graceful Shutdown Contract

The main function must register both `SIGTERM` and `SIGINT` handlers using `tokio::signal`:

```rust
let shutdown = async {
    tokio::select! {
        _ = tokio::signal::unix::signal(SignalKind::terminate())
              .expect("SIGTERM handler").recv() => {},
        _ = tokio::signal::ctrl_c() => {},
    }
};
```

**SIGTERM on macOS / Linux containers:** `tokio::signal::unix` is Linux/macOS only. Do not use it in cross-platform code paths. Since this binary runs exclusively on Ubuntu Linux inside Docker, `unix` signals are appropriate. The shutdown handler must be registered before the `tokio::select!` main loop begins — not inside a task — to ensure it is always active regardless of task state.

When the shutdown signal fires, the cancellation token is cancelled. Each task must respect the token and complete its shutdown sequence:

**Recorder Task shutdown:**
1. Stop reading new periods from ALSA.
2. Flush and close the current staging FLAC file (write any buffered bytes, fsync).
3. Move the staging file to `./recordings/` via `tokio::fs::rename`.
4. Log the final archive file path and size.

**Converter Task shutdown:** Drop the in-progress partial segment (log its incomplete frame count). Do not forward a partial segment to the Uploader via the mpsc channel.

**Cloud Uploader Task shutdown:**
1. Complete any in-flight `PUT` request (do not cancel mid-upload — would create a partial S3 object).
2. Write a final `manifest.json` with `"live": false` and `"updated_at": <now>`.
3. Exit.

**Timeout:** If any task has not exited within 25 seconds of the signal, `tokio::select!` in `main` forcefully aborts it and exits with code 1. This matches the 30-second Docker stop timeout with a 5-second buffer.
