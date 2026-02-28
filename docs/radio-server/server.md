# Server Crate

The `crates/server` library is the main binary that orchestrates the entire `radio-server` architecture. It initializes the Tokio runtime and runs four concurrent tasks.

## Shared State

All state is held in an `Arc<AppState>` and shared safely across tasks:

*   `streaming: AtomicBool`: Toggled by the `/start` and `/stop` HTTP endpoints.
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
3.  Creates a new, timestamped file in `./recordings/`.
4.  Enters an asynchronous loop, awaiting periods from the capture device.
5.  **For each period (4096 frames):**
    *   Computes the peak absolute sample value for the left and right channels for the UI. Updates `vu_left` and `vu_right`.
    *   Encodes the raw `&[i32]` buffer into raw verbatim HQ 24-bit FLAC frames.
    *   Writes the frames directly to the local archive file using `AsyncWriteExt::write_all`. Updates `recording_bytes`.
    *   Broadcasts the raw frames via the raw `tokio::sync::broadcast` channel to the converter process.
    *   Emits VU levels to `sse_tx`.
6.  On shutdown, it flushes the file and logs the final size.

### Process 2: Converter Task

Consumes the raw stream, normalizes it, and encodes it into multiple qualities (HQ and LQ).

1.  Subscribes to the raw broadcast channel.
2.  Initializes the `Normalizer`.
3.  Initializes the audio encoders: one `FlacEncoder` for the normalized HQ stream (24-bit), and one MP3 encoder (e.g., using a pure-Rust MP3 library set to 320kbps stereo) for the LQ stream.
4.  Loops, receiving raw FLAC frames and extracting the interleaved `i32` buffer.
5.  Copies the raw buffer to a new mutable buffer and calls `normalizer.process(&mut buffer)`.
6.  Encodes the normalized buffer simultaneously into the respective HQ (FLAC) and LQ (MP3) streams.
7.  Emits the current normalizer gain to `sse_tx` every 50ms using a `tokio::time::interval`.
8.  **Segment Assembly:** Accumulates the encoded frames for both qualities. When the target 10-second duration is reached (based on PCM sample count equivalent, `44100 * 2 channels * 4 bytes * 10s = 3,528,000` bytes):
    *   Assembles complete, standalone FLAC files in memory by prepending the respective stream headers.
    *   Broadcasts the completed HQ and LQ segment files (as `Bytes`) over dedicated segment channels to the Cloud Uploader.

### Process 3: Cloud Uploader Task

Receives completed segments and handles S3 uploads and manifest management.

1.  **Startup Cleanup:** Before processing any new segments, performs a `LIST` and `DELETE` operation on the bucket prefixes (`live/hq/` and `live/lq/`) to remove any orphaned segments from a previous crashed session.
2.  Subscribes to the completed HQ and LQ segment broadcast channels.
3.  Loops, receiving assembled segment files.
4.  **Upload:**
    *   Uploads the segments to S3 using raw HTTP with [AWS Signature V4](aws-sig-v4.md).
    *   **Resilience:** Uses an exponential backoff retry loop (e.g., 3-5 retries) for the HTTP PUT requests to handle transient network drops.
    *   Key format uses quality folders: e.g., `live/hq/segment-{:06}.flac` and `live/lq/segment-{:06}.mp3`.
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

**CRITICAL CONSTRAINT:** Two broadcast channels, not one. Raw frames go to the recorder. Normalized frames go to the R2 uploader. The local archive must be completely unprocessed — the normalizer must never touch the recorded audio.

**CRITICAL CONSTRAINT:** Rolling window, not TTL. R2 has no native TTL. The uploader maintains a VecDeque of uploaded keys and deletes the oldest immediately when the window exceeds 3 segments. At any moment R2 holds exactly 3 segments and one manifest.

**CRITICAL CONSTRAINT:** Segments are complete FLAC files. Every segment uploaded to R2 must be playable as a standalone FLAC file.