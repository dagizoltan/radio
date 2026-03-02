# Prompt for Session 2: Tokio Multi-Task Pipeline (The Engine)

**Goal:** Implement the main Tokio orchestration, data flow, threading, and downsampled multi-quality streaming for the `server` crate.

## 1. Project & State Setup (`crates/server/src/main.rs`)
1. Create the `server` crate and add `tokio`, `tokio-util`, `bytes`, `tracing`.
2. Define the global shared state exactly as:
   ```rust
   use bytes::Bytes;
   use std::collections::VecDeque;
   use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
   use tokio::sync::{broadcast, Mutex};

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
3. **Graceful Teardown Tree:** Use `tokio_util::sync::CancellationToken`.
   - Setup OS signal traps:
     ```rust
     let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
     tokio::select! {
         _ = sigterm.recv() => token.cancel(),
         _ = tokio::signal::ctrl_c() => token.cancel(),
     }
     ```
   - Await all spawned task `JoinHandle`s with a timeout:
     `tokio::time::timeout(Duration::from_secs(25), futures::future::join_all(handles)).await;`
     If the timeout fires, log a fatal error and `std::process::exit(1)`.

## 2. Channels Formulation
Initialize the specific bounded channels. Do not use unbound channels to prevent OOM on network stall.
- `let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Arc<Vec<i32>>>(16);`
- `let (seg_tx, mut seg_rx) = tokio::sync::mpsc::channel::<(u64, Bytes, Bytes)>(3);` // (Index, HQ, LQ)
- `let (sse_tx, _) = tokio::sync::broadcast::channel::<String>(32);`

## 3. Task 1: HQ Recorder (`src/recorder.rs`)
1. **Loop Initialization:**
   - Instantiate the ALSA `Device` from Session 1.
   - Instantiate a `FlacEncoder` configured for 48kHz, 2 channels, 24-bit, block size 4096.
   - Open a timestamped file: `./recordings/recording_{timestamp}.flac`.
   - Write the `FlacEncoder::stream_header()` bytes to the file immediately.
2. **Main Async Loop:**
   - Call the async `capture.read_period()` method.
   - **Metrics:** Calculate the peak absolute value of the left and right channels within the returned `Vec<i32>`. Update `state.vu_left` and `state.vu_right` (using `Ordering::Relaxed`). Send a fast JSON string `{"type":"vu", "l": val, "r": val}` to `sse_tx`.
   - **Archiving:** Pass the `Vec<i32>` slice to `encoder.encode_frame()`. Write the returned byte slice to the open `tokio::fs::File`. Update `state.recording_bytes`.
   - **Forwarding:** Wrap the `Vec<i32>` in an `Arc::new()` and send it to `pcm_tx`.
3. **Rotation:** Track the total frames written to the current file. When `total_frames >= 48000 * 3600` (1 hour):
   - `.flush().await` the current file.
   - Create a new timestamped file and instantiate a **new** `FlacEncoder` (resetting the frame counter to 0 for the new file).
   - Write the new `stream_header()`.
4. **Shutdown:** Break the loop if `token.is_cancelled()`. Always `.flush()` and close the file handle before returning.

## 4. Task 2: Converter (`src/converter.rs`)
1. **Initialization:**
   - Create `hq_encoder`: `FlacEncoder::new(48000, 2, 24, 4096)`.
   - Create `lq_encoder`: `FlacEncoder::new(24000, 2, 16, 2048)`.
   - CRITICAL: Call `hq_encoder.stream_header()` and acquire the `state.flac_header` Mutex to store it. The HTTP task relies on this.
2. **Pre-allocation:**
   - `let mut hq_accumulator = Vec::with_capacity(2_955_000);`
   - `let mut lq_accumulator = Vec::with_capacity(985_000);`
   - `let mut lq_staging = Vec::with_capacity(4096);` (To hold the decimated period).
3. **Main Rx Loop:** `while let Some(arc_pcm) = pcm_rx.recv().await`
   - **HQ Encode:** Call `hq_encoder.encode_frame(&arc_pcm)`. Append the returned bytes to `hq_accumulator`.
   - **LQ Downsample (Decimation & Dither):**
     - Iterate over the 4096 frames in `arc_pcm`. A stereo frame is 2 elements `[L, R]`.
     - We want to drop every other frame to halve the sample rate from 48kHz to 24kHz.
     ```rust
     lq_staging.clear();
     for i in (0..arc_pcm.len()).step_by(4) {
         let l_24 = arc_pcm[i];
         let r_24 = arc_pcm[i+1];
         // Arithmetic right shift by 8 bits converts 24-bit to 16-bit, preserving sign.
         let l_16 = l_24 >> 8;
         let r_16 = r_24 >> 8;
         lq_staging.push(l_16);
         lq_staging.push(r_16);
     }
     ```
     - **LQ Encode:** Call `lq_encoder.encode_frame(&lq_staging)`. Append bytes to `lq_accumulator`.
4. **Segment Assembly Logic:**
   - Track `frame_counter`. Add 4096 on each loop iteration.
   - When `frame_counter >= 491_520` (exactly 10.24 seconds, perfectly divisible by 4096):
     1. `let index = state.r2_segment.load(Ordering::SeqCst) + 1;` (Note: uploader actually manages the real index, so just pass a local monotonic counter or let the uploader assign the index).
     2. Convert accumulators to fast `Bytes`: `let hq_bytes = Bytes::from(hq_accumulator.clone());`
     3. Call `seg_tx.try_send((current_index, hq_bytes, lq_bytes))`. If `Err(TrySendError::Full)`, log `WARN: Uploader lagging, dropping segment`. DO NOT BLOCK.
     4. Clear the accumulators (retaining capacity): `hq_accumulator.clear()`.
     5. Reset `frame_counter = 0`.
     6. Increment `current_index`.

## Validation
Write a test binary that pushes a pre-generated 10-second `Vec<i32>` of pure sine wave data into the `pcm_rx` channel and verifies that `seg_rx` yields two `Bytes` objects: one ~2.9MB and one ~980KB, both valid standalone FLAC subframes (excluding stream header).
