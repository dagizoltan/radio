# Data Flow

This document traces the lifecycle of an audio sample through the Lossless Vinyl Radio Streaming System, from analog capture to browser playback.

## Audio Sample Lifecycle

1.  **ADC (Analog-to-Digital Conversion):** The Behringer UMC404HD converts the analog signal to digital (48000 Hz, 24-bit, stereo).
2.  **Kernel / ALSA:** The Linux kernel buffers the audio frames.
3.  **Capture (Rust - Process 1 HQ Recorder):** The [Capture Crate](./radio-server/capture.md) reads the audio frames from the ALSA device file into an interleaved `&mut [i32]` buffer via raw kernel `ioctl`s.
4.  **Raw Encoder (Process 1 HQ Recorder):** A raw FLAC [Encoder](./radio-server/encoder.md) takes the interleaved samples and produces raw verbatim HQ FLAC frames. The Recorder task writes them directly to the local archive.
5.  **Raw PCM Channel Send:** The raw PCM samples (interleaved `i32`, one period of 4096 frames) are cloned and sent to the Converter Task over a bounded `tokio::sync::mpsc` channel (capacity 16). The channel carries raw PCM, not encoded FLAC — the Recorder encodes to FLAC for its own archive write independently. This avoids any FLAC decode step in the Converter's hot path.
6.  **Multi-Quality Encode (Process 2 Converter):** The raw PCM samples received from the Recorder are encoded directly into an HQ verbatim FLAC stream and an LQ Opus stream (128 kbps stereo, Ogg container).
7.  **Segment Assembly & Broadcast:** The Converter task accumulates frames into 10-second segments, assembling complete standalone files (FLAC for HQ, continuous raw Opus for LQ) in memory. The LQ Opus stream uses an internal 960-sample staging buffer in the Converter Task to align ALSA periods (4096 frames) to valid Opus frame boundaries (960 frames at 20ms). At segment boundaries, any remaining frames are encoded exactly as-is; padding with silence is strictly forbidden to prevent audible clicks. Instead of an Ogg container, Opus packets are serialized into a simple custom binary format (e.g., a 2-byte length prefix followed by the payload) to enable gapless streaming and avoid `pre_skip` desync. It sends these assembled segments to the Cloud Uploader Task via a bounded `mpsc` channel (capacity 3).
8.  **S3 Upload (Process 3 Cloud Uploader):** The Uploader process receives the completed segments and pushes both the HQ and LQ files to S3 (MinIO or R2) via HTTP using robust S3 SDKs, employing exponential backoff retries for resilience. The `manifest.json` is updated to point to both streams.
9.  **Direct Segment Fetch:** The `radio-player` Web Component in the browser uses the manifest data to fetch the segment *directly* from the S3/R2 CDN edge (bypassing the Deno proxy to save bandwidth).
10. **Browser Decode:** The browser fetches the segment chunk-by-chunk using a `ReadableStream`. Each chunk is passed to the [WASM Decoder](./radio-client/wasm-decoder.md), which parses either the FLAC or Opus stream (depending on user selection) and yields `f32` PCM data.
11. **AudioWorklet Queue:** The `f32` PCM data is posted via `MessagePort` to the [AudioWorklet](./radio-client/worklet.md), which appends it to an internal ring buffer queue.
12. **Playback:** The `AudioWorkletProcessor` pulls 128 frames at a time from its queue, applies volume scaling, and outputs the stereo signal to the browser's audio destination, where it is finally converted back to analog by the listener's DAC and sent to the speakers.

## Two-Channel Pipeline Design

The `radio-server` audio pipeline uses two dedicated `tokio::sync::mpsc` channels for audio data, plus one `tokio::sync::broadcast` channel exclusively for control/SSE events. This is a critical design constraint.

### Raw PCM Channel (`mpsc`, capacity 16)
- **Producer:** The Recorder Task (sole ALSA reader).
- **Consumer:** The Converter Task (sole receiver).
- **Payload:** Raw interleaved PCM samples — `Arc<Vec<i32>>` or equivalent, one period (4096 frames) per message.
- **Why PCM, not FLAC:** The Recorder encodes the same PCM buffer to FLAC independently for the archive write. Sending PCM to the Converter means the Converter can re-encode to HQ FLAC and Opus directly, without a FLAC decode step in the hot path.
- **Back-pressure:** If the Converter falls behind, the bounded channel causes the Recorder's `send()` to return `Err`. The Recorder logs the drop and continues — one dropped period means ~85ms of silence in the archive, which is preferable to unbounded RAM growth.

### Segment Channel (`mpsc`, capacity 3)
- **Producer:** The Converter Task.
- **Consumer:** The Cloud Uploader Task (sole receiver).
- **Payload:** Completed 10-second segment files — `(SegmentIndex, HqBytes, LqBytes)`.
- **Back-pressure:** If the Uploader is behind, `try_send()` returns `Err` immediately. The Converter logs the dropped segment index and continues. The Recorder is never stalled by Uploader lag.

### SSE Event Bus (`broadcast`)
- **Producer:** Any task.
- **Consumers:** All connected monitor UI SSE clients (via `sse_tx`).
- **Payload:** JSON event strings (status, VU levels, recording info, R2 upload status).
- **Drop policy:** `RecvError::Lagged` from the broadcast channel is benign here — a monitor UI client that falls behind simply misses a VU meter update, not audio data.

**CRITICAL CONSTRAINT:** The Recorder encodes raw PCM to FLAC and writes it to the archive independently before forwarding PCM.

## Segment Lifecycle

The stream is chunked into discrete segments to enable low-latency HTTP delivery without WebSockets or specialized streaming protocols.

1.  **Accumulation:** The Converter Task accumulates encoded frames until 480,000 frames of PCM audio have been processed (10 seconds × 48000 Hz = 480,000 frames). Frame count, not byte count, is used as the threshold to remain independent of sample packing format (S24_LE, S32_LE, etc.). The Uploader Task receives complete pre-assembled segments — it does not accumulate frames itself.
2.  **File Creation:** The Converter Task assembles each quality's accumulated data into a complete, standalone file: for HQ, by prepending the cached FLAC stream header (`fLaC` marker + `STREAMINFO` block) to the accumulated verbatim frames; for LQ, by accumulating continuous raw Opus packets using a simple length-prefixed binary format. The Uploader receives these as fully-formed, ready-to-upload byte payloads.
3.  **Upload:** Segments are pushed to S3 with quality-namespaced, 8-digit zero-padded keys: `live/hq/segment-00000042.flac` for HQ FLAC and `live/lq/segment-00000042.opus` for LQ Opus.
4.  **Manifest Update:** `live/manifest.json` is overwritten with the new latest segment index.
5.  **Rolling Window:** The uploader maintains a queue of uploaded segment keys. If the window exceeds 10 segments, the oldest segment is explicitly deleted from S3 via a `DELETE` request.
6.  **Client Fetch:** Listeners fetch the `manifest.json` to find the live edge, then fetch the active segments sequentially.

**CRITICAL CONSTRAINT:** Rolling window, not TTL. R2 has no native TTL. The uploader maintains a `VecDeque` of uploaded keys and deletes the oldest immediately when the window exceeds 10 segments. At any moment R2 holds exactly 10 segments and one manifest.

### Segment Index Rollover

The monotonic segment index uses 8-digit zero-padding, supporting values from `00000000` to `99999999` (100 million segments). At 48000 Hz with 10-second segments, this boundary is reached after approximately **31.7 years** of continuous broadcast. When the index reaches `99999999`, it resets to `00000000` on the next segment.

**Server behaviour:** The Cloud Uploader Task wraps the index using modular arithmetic: `next_index = (current_index + 1) % 100_000_000`. The manifest's `latest` field reflects the post-wrap index.

**Client behaviour:** The fetch loop's jump-ahead logic must treat a rollover as a valid state. Detection condition: `latest < currentIndex && currentIndex - latest > 3`. On detection, snap to `latest` immediately — identical to the standard jump-ahead path.

**Startup:** On server restart, the uploader reads the highest existing segment index from the bucket (via `LIST` during the startup cleanup sweep) and resumes from that index, not from zero, preventing a spurious rollover event on routine restarts.
