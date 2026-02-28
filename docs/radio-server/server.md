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

## The Four Tokio Tasks

The `main` function executes four concurrent tasks using `tokio::select!`:

### 1. Pipeline Task

The core audio processing loop.

1.  Opens the ALSA capture device using the [Capture Crate](capture.md).
2.  Initializes two `FlacEncoder` instances (one raw, one normalized) and one `Normalizer`.
3.  Caches the `stream_header()` from both encoders. Sends the raw header down the raw broadcast channel and the normalized header down the normalized broadcast channel.
4.  Enters an asynchronous loop, awaiting periods from the capture device.
5.  **For each period (4096 frames):**
    *   Computes the peak absolute sample value for the left and right channels. Updates `vu_left` and `vu_right`.
    *   Encodes the raw `&[i16]` buffer using the raw encoder. Sends the resulting frame bytes down the raw broadcast channel.
    *   Copies the raw buffer to a new mutable buffer.
    *   Calls `normalizer.process(&mut buffer)`.
    *   Emits VU levels and the current normalizer gain to `sse_tx` every 50ms using a `tokio::time::interval`.
    *   Encodes the normalized buffer using the normalized encoder. Sends the resulting frame bytes down the normalized broadcast channel.

### 2. Recorder Task

Handles the local archiving of the stream.

1.  Subscribes to the raw broadcast channel.
2.  Creates a new, timestamped file in `./recordings/`.
3.  Loops, receiving `Bytes` (raw FLAC frames) from the channel.
4.  Writes every frame directly to the file using `AsyncWriteExt::write_all`.
5.  Updates `recording_bytes`.
6.  Spawns a background ticker task that emits a `recording` status event to `sse_tx` every 1 second.
7.  On channel close (e.g., shutdown), it flushes the file and logs the final size.

### 3. R2 Uploader Task

Handles assembling FLAC segments and uploading them to S3.

1.  Subscribes to the normalized broadcast channel.
2.  Loops, receiving `Bytes` (normalized FLAC frames).
3.  **Segment Assembly:** Accumulates frames into an internal `Vec<u8>`. The target size is equivalent to 10 seconds of raw PCM audio (approx. `44100 * 2 * 2 * 10 = 1,764,000` bytes). *Note: The actual accumulated FLAC bytes will be less, but the threshold is based on the PCM equivalent duration.*
4.  **Upload:** Once the 10-second threshold is reached:
    *   Assembles a complete, standalone FLAC file in memory by prepending the cached normalized stream header to the accumulated frames.
    *   Uploads the segment to S3 using raw HTTP with [AWS Signature V4](aws-sig-v4.md).
    *   Key format: `live/segment-{:06}.flac`.
    *   Writes `live/manifest.json` containing: `{"live": true, "latest": index, "segment_s": 10, "updated_at": timestamp}`.
5.  **Rolling Window:** Maintains a `VecDeque<String>` of uploaded S3 keys. If the deque length exceeds 3, it pops the oldest key and sends a `DELETE` request to S3.
6.  Pushes the new segment index and `Bytes` into `local_segments` (keeping only the last 3).
7.  Updates `r2_segment`, `r2_last_ms`, and `r2_uploading`. Emits `r2` status events to `sse_tx` during and after the upload.

### 4. HTTP Task

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