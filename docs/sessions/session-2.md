# Prompt for Session 2: Tokio Multi-Task Pipeline (The Engine)

**Goal:** Implement the main Tokio orchestration, data flow, threading, and downsampled multi-quality streaming.

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
- **Dual Encoders:** Initialize two persistent `FlacEncoder` instances.
  - `hq_encoder`: 48000 Hz, 2 channels, 24-bit, block size 4096.
  - `lq_encoder`: 24000 Hz, 2 channels, 16-bit, block size 2048.
  Ensure the HQ `FlacEncoder::stream_header()` is immediately acquired and written to `AppState.flac_header`.
- **LQ Downsampling Buffer:** Implement `lq_staging: Vec<i32>`. For each incoming `Arc<Vec<i32>>` (8192 elements representing 4096 frames):
  1. Iterate with step 4: `(0..received_arc.len()).step_by(4)` to skip every other frame.
  2. Take the Left and Right channel `i32` 24-bit samples: `let l = received_arc[i]; let r = received_arc[i+1];`
  3. Right-shift by 8 bits to convert 24-bit to 16-bit: `lq_staging.push(l >> 8); lq_staging.push(r >> 8);`
  4. Feed the `lq_staging` slice into `lq_encoder.encode_frame()`.
- **Segment Assembly:** Accumulate exactly 491,520 HQ frames (10.24 seconds = 120 ALSA periods). Track a `frame_counter`. Pre-allocate the HQ `Vec<u8>` to ~2,955,000 bytes and the LQ `Vec<u8>` to ~985,000 bytes.
- **Dispatch:** Upon reaching the 491,520-frame boundary:
  1. Package the completed HQ `Vec<u8>` (without stream header) and LQ `Vec<u8>` (without stream header).
  2. Use `try_send((index, Bytes::from(hq), Bytes::from(lq)))`.
  3. If full, drop the segment and log a warning.
  4. Reset accumulators and increment the internal index.

**Validation:**
Ensure the server correctly splits the signal, writes a continuous bit-perfect archive file to disk, and yields cleanly separated 10.24s HQ FLAC and LQ FLAC `Vec<u8>` buffers into the uploader channel without stalling or memory leaking.
