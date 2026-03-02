# Prompt for Session 2: Tokio Multi-Task Pipeline (The Engine)

**Goal:** Implement the main Tokio orchestration, data flow, threading, and continuous Opus streaming.

**Context & Requirements:**
You will build the `server` crate's core pipeline, orchestrating the `capture` and `encoder` crates built in Session 1, and introducing the `Converter Task`.

**1. Main Tokio Orchestration (`main.rs`):**
- Set up a single `tokio::select!` runtime managing the three primary tasks (Recorder, Converter, Uploader).
- **Graceful Shutdown:** Implement robust `SIGTERM`/`SIGINT` handlers using `tokio::signal::unix`. When triggered, cancel a shared `tokio_util::sync::CancellationToken`. Await all task JoinHandles. If they don't exit within 25 seconds, use `tokio::time::timeout` to forcefully abort the process with code 1.
- **Shared State:** Define the `AppState` struct:
  ```rust
  pub struct AppState {
      pub streaming: AtomicBool,
      pub vu_left: AtomicI32,
      pub vu_right: AtomicI32,
      pub recording_path: Mutex<String>,
      pub recording_bytes: AtomicU64,
      pub r2_segment: AtomicU64,
      pub r2_last_ms: AtomicU64,
      pub r2_uploading: AtomicBool,
      pub local_segments: Mutex<VecDeque<(u64, Bytes)>>,
      pub flac_header: Mutex<Option<Bytes>>,
      pub sse_tx: broadcast::Sender<String>,
  }
  ```

**2. Channels & Task 1 (HQ Recorder Task):**
- Create two dedicated `tokio::sync::mpsc` channels for audio:
  - `mpsc::channel::<Arc<Vec<i32>>>(16)` for zero-copy raw PCM routing (Recorder -> Converter).
  - `mpsc::channel::<(u64, Bytes, Bytes)>(3)` for assembled segments `(index, hq_bytes, lq_bytes)` (Converter -> Uploader).
- Wire the `capture` crate loop into Task 1. On every period, update `vu_left` and `vu_right`, encode the FLAC frames for the local archive file directly to disk, then send the `Arc<Vec<i32>>` to the Converter.
- Ensure the archive file rotates exactly every hour by tracking elapsed frames. Close and fsync the old file, open a new timestamped file, and write a fresh `stream_header()` to it.

**3. Task 2 (Converter Task):**
- **Dual Encoders:** Initialize a persistent `FlacEncoder` for HQ and an `audiopus::Encoder` for LQ (48000 Hz, Stereo, Application::Audio, 128000 bps unconstrained VBR). Ensure the `FlacEncoder::stream_header()` is immediately acquired and written to `AppState.flac_header`.
- **Opus Staging Buffer:** Implement `opus_staging: Vec<i32>`. For each incoming `Arc<Vec<i32>>` (8192 elements):
  1. `opus_staging.extend_from_slice(&received_arc);`
  2. Loop over `opus_staging.chunks_exact(1920)` (960 stereo frames).
  3. Convert the chunk to `f32` (`sample as f32 / 8388608.0`), encode via `audiopus`, and append to the LQ `Vec<u8>`.
  4. Extract the remainder, overwrite the front of `opus_staging`, and truncate to the remainder length (one memory shift per period).
- **Segment Assembly:** Accumulate exactly 491,520 frames (10.24 seconds = 120 ALSA periods = 512 Opus frames). Track a `frame_counter`. Pre-allocate the HQ `Vec<u8>` to ~2,955,000 bytes.
- **Opus Gapless Formatting:** Serialize the raw Opus packets continuously by prepending each packet with a 2-byte Big Endian payload length prefix. Do NOT use an Ogg container. Do NOT reset the Opus encoder state between segments.
- **Dispatch:** Upon reaching the 491,520-frame boundary:
  1. Package the completed HQ `Vec<u8>` (without stream header) and LQ `Vec<u8>`.
  2. Use `try_send((index, Bytes::from(hq), Bytes::from(lq)))`.
  3. If full, drop the segment and log a warning.
  4. Reset accumulators and increment the internal index.

**Validation:**
Ensure the server correctly splits the signal, writes a continuous bit-perfect archive file to disk, and yields cleanly separated 10.24s FLAC and Opus `Vec<u8>` buffers into the uploader channel without stalling or memory leaking.
