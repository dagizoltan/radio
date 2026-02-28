# Data Flow

This document traces the lifecycle of an audio sample through the Lossless Vinyl Radio Streaming System, from analog capture to browser playback.

## Audio Sample Lifecycle

1.  **ADC (Analog-to-Digital Conversion):** The Behringer UMC404HD converts the analog signal to digital (44100 Hz, 16-bit, stereo).
2.  **Kernel / ALSA:** The Linux kernel buffers the audio frames.
3.  **Capture (Rust):** The [Capture Crate](../radio-server/capture.md) reads the audio frames from the ALSA device file into an interleaved `&mut [i16]` buffer via raw kernel `ioctl`s when the file descriptor becomes readable.
4.  **Raw Encoder:** A raw FLAC [Encoder](../radio-server/encoder.md) takes the interleaved samples and produces raw verbatim FLAC frames.
5.  **Raw Channel Broadcast:** The raw FLAC frames are broadcast over the raw `tokio::sync::broadcast` channel. The Recorder task receives these frames and writes them directly to the local archive.
6.  **Normalization:** The original interleaved `i16` buffer is passed to the [Normalizer](../radio-server/normalizer.md), which converts the samples to `f32`, applies LUFS gain riding and true-peak limiting, and converts them back to clamped `i16` in-place.
7.  **Normalized Encoder:** A second FLAC [Encoder](../radio-server/encoder.md) takes the normalized samples and produces normalized verbatim FLAC frames.
8.  **Normalized Channel Broadcast:** The normalized FLAC frames are broadcast over the normalized `tokio::sync::broadcast` channel. The R2 Uploader task receives these frames.
9.  **Segment Assembly:** The R2 Uploader accumulates normalized FLAC frames in memory until it reaches 10 seconds of audio. It prepends the FLAC stream header to create a standalone FLAC file in memory.
10. **S3 Upload:** The assembled segment is uploaded to S3 (MinIO or R2) via raw HTTP using [AWS Signature V4](../radio-server/aws-sig-v4.md). The `manifest.json` is updated.
11. **Proxy Fetch:** The `radio-client` proxies the segment request from the browser to S3.
12. **Browser Fetch & Decode:** The `radio-player` Web Component in the browser fetches the segment chunk-by-chunk using a `ReadableStream`. Each chunk is passed to the [WASM Decoder](../radio-client/wasm-decoder.md), which parses the FLAC stream and yields `f32` PCM data.
13. **AudioWorklet Queue:** The `f32` PCM data is posted via `MessagePort` to the [AudioWorklet](../radio-client/worklet.md), which appends it to an internal ring buffer queue.
14. **Playback:** The `AudioWorkletProcessor` pulls 128 frames at a time from its queue, applies volume scaling, and outputs the stereo signal to the browser's audio destination, where it is finally converted back to analog by the listener's DAC and sent to the speakers.

## Two Broadcast Channel Design

The `radio-server` utilizes two distinct `tokio::sync::broadcast` channels. This is a critical design constraint to guarantee the local archive remains an unadulterated copy of the captured signal.

*   **Raw Channel:** Carries `Bytes` of unprocessed, raw FLAC frames.
    *   **Subscribers:** The Recorder Task.
*   **Normalized Channel:** Carries `Bytes` of normalized FLAC frames.
    *   **Subscribers:** The R2 Uploader Task.

The normalizer processes samples in a mutable buffer *after* the raw frames have been encoded and dispatched, ensuring the original capture path is never touched by the gain rider or limiter.

**CRITICAL CONSTRAINT:** The normalizer must never touch the recorded audio. Two broadcast channels, not one. Raw frames go to the recorder. Normalized frames go to the R2 uploader.

## Segment Lifecycle

The stream is chunked into discrete segments to enable low-latency HTTP delivery without WebSockets or specialized streaming protocols.

1.  **Accumulation:** The R2 Uploader task accumulates FLAC frames until the equivalent of 10 seconds of raw PCM audio is reached (1,764,000 bytes).
2.  **File Creation:** The segment is assembled into a complete, standalone FLAC file by prepending the cached FLAC stream header (`fLaC` marker + `STREAMINFO` block).
3.  **Upload:** The segment is pushed to S3 with a key like `live/segment-000042.flac`.
4.  **Manifest Update:** `live/manifest.json` is overwritten with the new latest segment index.
5.  **Rolling Window:** The uploader maintains a queue of uploaded segment keys. If the window exceeds 3 segments, the oldest segment is explicitly deleted from S3 via a `DELETE` request.
6.  **Client Fetch:** Listeners fetch the `manifest.json` to find the live edge, then fetch the active segments sequentially.

**CRITICAL CONSTRAINT:** Rolling window, not TTL. R2 has no native TTL. The uploader maintains a `VecDeque` of uploaded keys and deletes the oldest immediately when the window exceeds 3 segments. At any moment R2 holds exactly 3 segments and one manifest.