# Prompt for Session 2: Tokio Multi-Task Pipeline (The Engine)

**Goal:** Implement the main Tokio orchestration, data flow, threading, and continuous Opus streaming.

**Context & Requirements:**
You will build the `server` crate's core pipeline, orchestrating the `capture` and `encoder` crates built in Session 1, and introducing the `Converter Task`.

**1. Main Tokio Orchestration (`main.rs`):**
- Set up a single `tokio::select!` runtime managing the three primary tasks (Recorder, Converter, Uploader).
- **Graceful Shutdown:** Implement robust `SIGTERM`/`SIGINT` handlers using `tokio::signal::unix`. When triggered, cancel a shared `CancellationToken` to initiate graceful shutdown across all tasks. Wait up to 25 seconds for tasks to exit before forcefully aborting.
- **Shared State:** Define the `Arc<AppState>` containing metrics (`vu_left`, `vu_right`, `r2_segment`, `r2_uploading`), the `local_segments` `VecDeque` rolling window, and the `flac_header` Mutex.

**2. Channels & Task 1 (HQ Recorder Task):**
- Create two dedicated `tokio::sync::mpsc` channels for audio: `mpsc(16)` for raw PCM (Recorder -> Converter) and `mpsc(3)` for assembled segments (Converter -> Uploader).
- Wire the `capture` crate loop into Task 1. On every period, encode the FLAC frames for the local archive file directly to disk, then send an `Arc<Vec<i32>>` of the raw PCM to the Converter.
- Ensure the archive file rotates hourly.

**3. Task 2 (Converter Task):**
- **Dual Encoders:** Initialize a persistent `FlacEncoder` for HQ and an `audiopus::Encoder` for LQ (48000 Hz, Stereo, Application::Audio, 128000 bps unconstrained VBR). Ensure the `FlacEncoder` stream header is immediately written to `AppState.flac_header`.
- **Opus Staging Buffer:** Implement the `opus_staging` buffer logic to collect 4096-frame ALSA periods and chunk them into exactly 960-frame slices for the `libopus` encoder.
- **Segment Assembly:** Accumulate exactly 491,520 frames (10.24 seconds = 120 ALSA periods = 512 Opus frames). Pre-allocate the HQ `Vec<u8>` to ~2,955,000 bytes to avoid mid-segment reallocations.
- **Opus Gapless Formatting:** Serialize the raw Opus packets continuously by prepending each packet with a 2-byte Big Endian payload length prefix. Do NOT use an Ogg container. Do NOT reset the Opus encoder state between segments.
- **Dispatch:** Upon crossing the 491,520-frame threshold, package the completed HQ (without stream header prepended) and LQ segments and `try_send` them to the Uploader channel.

**Validation:**
Ensure the server correctly splits the signal, writes a continuous bit-perfect archive file to disk, and yields cleanly separated 10.24s FLAC and Opus `Vec<u8>` buffers into the uploader channel without stalling or memory leaking.
