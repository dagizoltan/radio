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
3.  Creates a new, timestamped staging file directly in the host-mounted volume at `./recordings/recording-{timestamp}.flac`. The `tmpfs` staging approach was removed because it poses a permanent data loss risk in the event of a power failure or Docker crash. A standard SSD can easily handle the 1.5 GB/hour continuous append-only write workload.
4.  Enters an asynchronous loop, awaiting periods from the capture device.
5.  **For each period (4096 frames):**
    *   Calls `IOCTL_READI_FRAMES` to read the period into an `&mut [i32]` buffer.
    *   **XRUN handling (check first):** If `IOCTL_READI_FRAMES` returns `EPIPE` (ALSA buffer overrun), skip all steps below, call `IOCTL_PREPARE` to reset the hardware, increment `radio_capture_overruns_total`, log a `WARN`, and loop back to `async_fd.readable()`. See the [Capture Crate XRUN Recovery](capture.md#xrun-recovery-epipe-handling) section for the full recovery sequence. The following steps only execute on a successful read (`result > 0`).
    *   Computes the peak absolute sample value for the left and right channels for the UI. Updates `vu_left` and `vu_right`.
    *   Encodes the raw `&[i32]` buffer into raw verbatim HQ 24-bit FLAC frames.
    *   Writes the frames directly to the archive file using `AsyncWriteExt::write_all`. Updates `recording_bytes`.
    *   Sends the raw PCM period as `Arc<Vec<i32>>` via a bounded `tokio::sync::mpsc` channel (capacity 16) to the Converter Task. The FLAC encoding above is written to the archive only — the Converter receives raw PCM directly and never sees the FLAC-encoded bytes.
    *   Emits VU levels to `sse_tx`.
6.  **Archiving:** Periodically (every 8 hours, or on graceful shutdown), it flushes and closes the current archive file to prevent excessively large file sizes, and opens a new timestamped file to continue.

### Process 2: Converter Task

Consumes the raw stream and encodes it into multiple qualities (HQ and LQ).

1.  Receives from the bounded `mpsc` channel. The `broadcast` channel carries only JSON event strings for the monitor UI SSE feed. Shutdown is coordinated via a cancellation token, not via the broadcast channel.
2.  Initializes the audio encoders: one `FlacEncoder` for the HQ stream (24-bit), and one Opus encoder (`audiopus::Encoder` configured for `48000 Hz`, `Stereo`, `Application::Audio`, bitrate `128000 bps`). Before encoding for Opus, samples are converted from `i32` to `f32` (range `[-1.0, 1.0]`). The Opus encoder produces variable-length packets. Rather than an Ogg container, these raw packets are appended directly into the segment's byte buffer, prefixed by their 2-byte payload length in network byte order (Big Endian), allowing the decoder to slice a continuous stream of Opus packets perfectly gaplessly across 10-second segments.

**Opus PCM Staging Buffer**

Before any samples are passed to the Opus encoder, they must be staged through an internal PCM buffer local to the Converter Task:

```
opus_staging: Vec<i32>  // interleaved stereo, pre-allocated with capacity 960 * 2
```

On each received ALSA period (4096 interleaved stereo frames = 8192 `i32` values):
1. Append all samples from the received `Arc<Vec<i32>>` into `opus_staging`.
2. While `opus_staging.len() >= 1920` (960 frames × 2 channels):
   - Drain the first 1920 samples from `opus_staging`.
   - Convert the drained `i32` samples to `f32` (divide by `8388608.0` for 24-bit).
   - Pass the 1920-sample `f32` slice to `opus_encoder.encode_float()`.
   - Accumulate the resulting Opus packet into the current segment's Ogg writer.
3. Leftover samples (at most 1919) remain in `opus_staging` for the next period.

**Frame size constraint:** The Opus encoder must be initialized with a frame size of `960` samples. Do not use other valid Opus frame sizes (480, 2880, etc.) — 960 (20ms at 48kHz) is the standard and produces the best quality/latency balance for this use case.

**Segment boundary:** When the 480,000-frame threshold is reached and a segment is finalized, flush `opus_staging` by passing the exact number of remaining frames directly to the encoder. **Do not** pad with silence, as this will introduce an audible click at the end of every LQ segment. Pass this exact, smaller slice (e.g., 400 frames instead of 960) to `opus_encoder.encode_float()`. The continuous encoder state must **not** be reset. The resulting raw packet is simply prefixed by its 2-byte length and appended to the final segment. Reset `opus_staging` to empty for the next segment.

3.  Loops, receiving raw PCM periods from the Recorder Task via the bounded `mpsc` channel. Each message is an `Arc<Vec<i32>>` containing one period of 4096 interleaved frames. For each received period:
    *   **HQ FLAC path:** `encode_frame()` receives a `&[i32]` borrow of the `Arc` contents directly. Zero copy, zero allocation.
    *   **LQ Opus path:** Samples are copied element-wise into `opus_staging` for frame-size alignment (see Opus PCM Staging Buffer above). This is a necessary copy; the shared `Arc` contents are never mutated.
    *   Neither path mutates the shared buffer. The `Arc` is dropped when both paths have consumed the period.
4.  **Segment Assembly:** Accumulates the encoded frames for both qualities.
    *   *Optimization Strategy:* To prevent constant vector reallocations as frames accumulate, the accumulator `Vec<u8>` for the 10-second segment must be pre-allocated with `Vec::with_capacity()`. A 10-second 24-bit verbatim FLAC segment contains the raw PCM payload (`48000 * 3 bytes * 2 channels * 10s = 2,880,000` bytes) plus the FLAC framing overhead. A 10s segment has ~117 FLAC frames, each contributing a frame header (~10-15 bytes) and a CRC-16 (2 bytes), adding around ~1,700 bytes. To ensure zero reallocations, the buffer must be padded slightly larger than just the PCM size.
    *   **Threshold:** 480,000 frames (10 × 48000 Hz). Track a `frame_counter: u64` incremented by `period_frames` (4096) per ALSA period. Pre-allocate accumulator: `Vec::with_capacity(2_885_000)` for HQ (PCM + framing overhead padding).
    *   Assembles complete, standalone files in memory by prepending the respective stream headers to the filled accumulator.
    *   Sends completed HQ and LQ segment files to the Cloud Uploader via a bounded `tokio::sync::mpsc` channel with capacity **3**. Uses `try_send()` (non-blocking). If the channel is full, logs `WARN: segment {index} dropped — uploader lagging` and discards the segment. Never blocks waiting for the Uploader.

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
    *   **Resilience:** Uses an exponential backoff retry loop (e.g., 3-5 retries) for the HTTP PUT requests to handle transient network drops. To prevent unbounded channel stalls from backing up the pipeline, the `reqwest` client must be configured with aggressive timeouts (e.g., a `connect_timeout` of 2s and a total `timeout` of 8s). If the HTTP request hits the timeout, it counts as a failure towards the retry limit. Once exhausted, the segment is aggressively dropped, unblocking the channel so the Recorder task is not impacted.
    *   Key format uses quality folders: e.g., `live/hq/segment-{:08}.flac` and `live/lq/segment-{:08}.opus`.
    *   Writes `live/manifest.json` containing metadata for both streams: `{"live": true, "latest": index, "segment_s": 10, "updated_at": timestamp, "qualities": ["hq", "lq"]}`.
    *   **Crucial Caching Instruction:** The `PUT` request for `manifest.json` **must** include the `Cache-Control: no-store, max-age=0` metadata explicitly. Although `no-cache` would allow ETag-based revalidation, clients implement ETag tracking manually in JavaScript memory and fetch directly from R2. `no-store` is used to ensure no intermediate CDN layer retains any copy of the manifest.

    **Retry exhaustion:** If all retries are exhausted for a segment `PUT`:
    1. Log `ERROR: segment {index} upload failed after {n} retries — skipping`.
    2. Increment the `radio_segment_upload_exhaustion_total` Prometheus metric.
    3. Discard the segment bytes from memory.
    4. Do **not** update the manifest. The `latest` index does not advance for the failed segment.
    5. Do **not** update the rolling window queue — the failed segment was never successfully stored on R2, so there is nothing to delete.
    6. Continue processing the next segment received from the Converter channel.
    7. Emit an `r2` SSE event with `{ "uploading": false, "segment": index, "last_ms": <last_successful_ms>, "error": true }` so the monitor UI can surface the failure.

    **Client behavior under skipped indices:** The client's jump-ahead logic already handles gaps. If `latest` jumps by 2 (a segment was skipped), the client detects `currentIndex < latest - 3` and snaps to `latest - 1`. No client-side changes are needed.

    **Manifest retry:** The manifest `PUT` uses the same exponential backoff. If the manifest `PUT` fails after all retries, log `ERROR: manifest update failed for segment {index}`. The segment data is already uploaded to R2 (the segment `PUT` succeeded). The `latest` pointer in the manifest simply does not advance. On the next segment cycle, the manifest will be overwritten with the new `latest`. Clients experiencing a stale manifest will detect it via the `updated_at` staleness check (`Date.now() - updated_at > segment_s * 3 * 1000`) and show "Stream may be offline."

5.  **Rolling Window:** Maintains a queue of uploaded S3 keys for all qualities. If the window exceeds 10 segments per quality, it issues `DELETE` requests for the oldest objects. A 10-segment window is strictly required to prevent intermittent 404 errors for clients recovering from lag or fetching immediately after a manifest update.
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

**CRITICAL CONSTRAINT:** Two dedicated audio `mpsc` channels, not broadcast. Raw PCM flows Recorder→Converter via `mpsc(16)`. Assembled segments flow Converter→Uploader via `mpsc(3)`. The `broadcast` channel carries only SSE event bus messages.

**CRITICAL CONSTRAINT:** Rolling window, not TTL. R2 has no native TTL. The uploader maintains a VecDeque of uploaded keys and deletes the oldest immediately when the window exceeds 10 segments. At any moment R2 holds exactly 10 segments and one manifest.

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
2. Flush and close the current FLAC archive file (write any buffered bytes, fsync) to ensure the file is completely valid and flushed to disk before the process exits.
3. Log the final archive file path and size.

**Converter Task shutdown:** Drop the in-progress partial segment (log its incomplete frame count). Do not forward a partial segment to the Uploader via the mpsc channel. **CRITICAL:** Explicitly drop the `mpsc::Sender` instance targeting the Uploader Task when the Converter exits. This ensures the Uploader's `recv()` call wakes up and returns `None`, allowing it to perform its own graceful shutdown without deadlocking.

**Cloud Uploader Task shutdown:**
1. Complete any in-flight `PUT` request (do not cancel mid-upload — would create a partial S3 object).
2. Write a final `manifest.json` with `"live": false` and `"updated_at": <now>`.
3. Exit.

**Timeout:** If any task has not exited within 25 seconds of the signal, `tokio::select!` in `main` forcefully aborts it and exits with code 1. This matches the 30-second Docker stop timeout with a 5-second buffer.
