# Design Decisions

This document outlines the rationale behind the key technical decisions in the Lossless Vinyl Radio Streaming System.

## Why raw ioctls and not libasound?

The [Capture Crate](../radio-server/capture.md) interacts directly with the Linux kernel via raw ALSA `ioctl`s instead of using `libasound2` (the standard C ALSA library) or Rust bindings to it.

*   **Rationale:** Eliminating C dependencies simplifies the build process, cross-compilation, and runtime environment. It removes an entire class of potential linking issues and segfaults. By using `rustix` for safe, zero-cost syscall wrappers, we achieve direct kernel communication in pure Rust, ensuring memory safety and deterministic behavior.
*   **Constraint:** No C bindings in capture. The capture crate must not link against `libasound` or any C audio library.

## Why two broadcast channels?

The server pipeline uses two separate `tokio::sync::broadcast` channels: one for raw audio and one for normalized audio.

*   **Rationale:** The local archive must be a pristine, unprocessed bit-for-bit copy of the analog capture for archival purposes. The stream sent to listeners needs to be normalized (gain-ridden and limited) to provide a consistent listening experience without digital clipping. By splitting the pipeline immediately after the raw encode, we guarantee the [Normalizer](../radio-server/normalizer.md) never mutates the archived audio.
*   **Constraint:** Two broadcast channels, not one. Raw frames go to the recorder. Normalized frames go to the R2 uploader. The normalizer never touches the recorded audio.

## Why verbatim FLAC subframes?

The [Encoder](../radio-server/encoder.md) only outputs verbatim FLAC subframes (uncompressed PCM wrapped in FLAC framing), bypassing LPC (Linear Predictive Coding) and Rice coding.

*   **Rationale:** Producing verbatim subframes dramatically simplifies the encoder and decoder implementations. It eliminates the need for complex mathematical modeling and prediction algorithms in the hot path. While the files are larger than fully compressed FLAC, they remain perfectly compliant with the standard FLAC specification and retain the essential benefits: self-contained framing, robust sync codes, and checksums (CRC-8/CRC-16).
*   **Future Enhancement Note:** To balance the massive gain in simplicity with bandwidth efficiency (server to cloud), a lightweight FLAC compression strategy—such as basic Rice coding or a lower-level compression preset—can be introduced. This would reduce the payload size by 15-30% without significantly increasing CPU overhead on the encoder or decoder.

## Why a custom WASM decoder?

The browser uses a custom Rust-compiled [WASM Decoder](../radio-client/wasm-decoder.md) to decode the FLAC stream.

*   **Rationale:** While the Web Audio API provides `AudioContext.decodeAudioData()`, it expects complete files and cannot decode a continuous stream of chunks as they arrive over HTTP. We need to stream the audio chunk-by-chunk to the `AudioWorklet` to maintain low latency. A minimal WASM decoder specifically tailored to our verbatim FLAC subset is incredibly fast, lightweight, and allows for precise streaming control.

## Why AudioWorklet and not ScriptProcessorNode?

The browser playback relies on an [AudioWorklet](../radio-client/worklet.md) to output the audio.

*   **Rationale:** `ScriptProcessorNode` is deprecated and runs on the main JavaScript thread. This means any UI rendering, garbage collection, or other main-thread activity can cause audio dropouts and glitches. `AudioWorklet` runs in a dedicated audio rendering thread, providing a robust, glitch-free audio output mechanism decoupled from the main UI thread.
*   **Constraint:** AudioWorklet for audio output. The browser audio player must use `AudioWorkletNode`, not `ScriptProcessorNode`.

## Why path-style S3 URLs?

The server and client use path-style S3 URLs (`{endpoint}/{bucket}/{key}`) instead of virtual-hosted-style URLs (`{bucket}.{endpoint}/{key}`).

*   **Rationale:** Path-style URLs work seamlessly across both MinIO (local development) and Cloudflare R2 (production) without requiring complex DNS configurations or custom hostname mappings in local environments.
*   **Constraint:** Path-style S3 URLs. All S3 operations must use path-style.

## Why a rolling window and not TTL?

The R2 Uploader actively maintains a queue of uploaded segments and issues `DELETE` requests for the oldest segments.

*   **Rationale:** Cloudflare R2 does not have a native, immediate Time-to-Live (TTL) feature that deletes objects precisely when they expire. To prevent unbounded storage growth for a continuous live stream, the server must manage the lifecycle manually. A rolling window ensures exactly 3 segments are stored per quality stream at any time.
    *   *Startup Cleanup:* To prevent "orphaned" segments resulting from an abrupt crash where the rolling window queue in RAM is lost, the server executes a one-time "cleanup sweep" of the S3 bucket prefixes (`live/hq/` and `live/lq/`) upon startup, issuing `DELETE`s for all existing segments before resuming broadcast.
    *   *Robust Cleanup Fallback:* In production, this manual rolling window is heavily supplemented by cloud provider S3 Object Lifecycle Rules (e.g., Cloudflare R2 bucket policies) configured to automatically delete segments older than a few minutes. This acts as a robust safety net against storage leaks and orphaned files if the server process crashes ungracefully.
*   **Constraint:** Rolling window, not TTL. The uploader maintains a `VecDeque` of uploaded keys and deletes the oldest immediately when the window exceeds 3 segments, while relying on the cloud platform's bucket policy for an ultimate backup.

## Why a Custom Chunk-Streaming Architecture (Why not HLS/Icecast)?

The system builds its own segmented streaming protocol (a JSON manifest pointing to standalone 10-second FLAC/MP3 files) rather than using industry standards like Icecast or HTTP Live Streaming (HLS).

*   **Rationale:**
    *   *Vs. Icecast:* Icecast requires a long-lived, dedicated TCP connection from every listener to a central server. This scales poorly and is vulnerable to transient network drops. Our architecture pushes static chunks to an edge CDN (Cloudflare R2), making delivery infinitely scalable, cacheable, and resilient to client network hiccups.
    *   *Vs. HLS/MPEG-DASH:* Modern platforms chunk media into segments (like we do) and use an `.m3u8` playlist. Browsers play these chunks natively using the Media Source Extensions (MSE) API. **However, browsers generally do not support FLAC via MSE.** If we used standard HLS, we would be forced to use lossy codecs (AAC/MP3) for everything, sacrificing our primary 24-bit lossless archival and broadcast goal. By building a custom chunk-fetcher, a WASM decoder, and an `AudioWorklet`, we completely bypass the browser's native codec limitations.
    *   *Chunk Size vs. Latency:* While 10-second chunks inherently introduce a high latency of 10-20 seconds (the time to encode, upload, and for the client to fetch), the primary goal of this system is **production-grade, high-quality audio stability**, not ultra-low latency. Using 10-second segments significantly reduces the volume of HTTP PUT/GET requests, lowering overall system stress and making client-side playback alignment via WebAssembly far more reliable with fewer gapless playback transitions to compute.

## Why MP3 for the LQ stream?

The Converter process explicitly encodes the lower-quality (LQ) fallback stream as a high-resolution, stereo MP3 (e.g., 320kbps).

*   **Rationale:** To provide a viable lower-bandwidth fallback (compared to the heavy 24-bit lossless FLAC) without breaking the "clean Rust architecture" rule, we need an encoder that is available in pure Rust (or highly portable and easily vendored). MP3 offers excellent compatibility and significant bandwidth reduction while maintaining full stereo width (vs downsampling PCM). A lightweight pure-Rust WASM MP3 decoder (like `minimp3-rs`) can be bundled alongside the FLAC decoder, keeping the `AudioWorklet` chunk-streaming architecture perfectly identical for both formats.

## Why explicitly flush the AudioWorklet on quality switch?

When a listener toggles the Quality Selector (HQ ↔ LQ), the player immediately flushes the `AudioWorklet`.

*   **Rationale:** The `AudioWorklet` queue buffers `f32` PCM arrays. If the user switches qualities, there might be 1-2 seconds of HQ audio left in the queue. Appending LQ-decoded chunks directly behind the pending HQ chunks (especially if a codec introduced slight padding/delay differences) can cause an audible pop or phase alignment jump. By sending a `FLUSH` command via `postMessage`, the worklet clears its queue, ensuring a clean break and a glitch-free transition to the new stream quality.

## Why use a Ring Buffer in the AudioWorklet?

The `AudioWorklet` manages its internal PCM state using a single, pre-allocated `Float32Array` acting as a circular/ring buffer, rather than a standard JavaScript array that `push()`es and `shift()`s incoming chunks.

*   **Rationale:** The V8 JavaScript Garbage Collector (GC) runs periodically to clean up discarded objects. In high-performance, real-time audio, allocating new arrays for every incoming chunk and discarding them after playback causes frequent GC pauses. Even a pause of a few milliseconds on the audio thread results in an audible click or dropout. A pre-allocated ring buffer performs **zero allocations** during playback. Data is written to the ring, and the read pointer advances, ensuring a perfectly smooth, glitch-free audio stream.

## Handling the "Tunnel" Scenario (Reconnection)

If a mobile listener loses their internet connection (e.g., entering a tunnel) for 45 seconds, the `radio-player` fetch loop will fail.

*   **Rationale:** The player must not crash or permanently play 45 seconds behind live when the connection is restored. The fetch loop catches `fetch()` exceptions and enters an incremental backoff retry state. Once reconnected, it polls the `manifest.json`. If it detects the player's `currentIndex` is drastically behind the manifest's `latest` index (e.g., `currentIndex < latest - 3`), it invokes the "Jump-Ahead Logic" to instantly snap back to the live edge, dropping the missed segments and resuming real-time playback seamlessly.

## CDN Edge Caching Strategy

The S3 Uploader explicitly sets `Cache-Control` headers when pushing to Cloudflare R2 to ensure the CDN scales infinitely without hitting the origin bucket.

*   **Rationale:** If a thousand listeners request the same 10-second chunk simultaneously, the edge CDN must serve it to prevent high egress costs and bucket throttling.
*   **Constraint:** FLAC/MP3 segments must be uploaded with `Cache-Control: public, max-age=31536000, immutable` (they never change once uploaded). The `manifest.json` must be uploaded with `Cache-Control: no-store, max-age=0` (it must always be fetched fresh to find the newest chunks).

## Clock Drift and Buffer Management

The browser client implements dynamic buffer management to counteract internal clock drift between the ThinkPad recording the audio and the listener's browser playing it.

*   **Rationale:** If the browser plays audio slightly faster than the server encodes 10-second chunks, the browser will eventually run out of data and stall. Conversely, if it plays slower, the buffer will bloat and latency will increase infinitely.
*   **Implementation:** The client monitors its internal buffer depth. If it detects drift accumulating beyond safe thresholds, it will either silently drop an old chunk (if drifting too far behind) or induce a brief pause to allow the buffer to refill (if playing too fast), ensuring long-term sync stability.

## CORS Security Policy

The cloud storage bucket (R2/MinIO) enforces a strict Cross-Origin Resource Sharing (CORS) policy.

*   **Rationale:** Because the `radio-client` Web Component fetches segments directly from the bucket (to bypass the Deno proxy and save bandwidth), the bucket must explicitly allow cross-origin `GET` requests from the Deno Deploy frontend URL, preventing unauthorized embedding or leeching from other domains.

## Archival Storage Rotation

The ThinkPad running the `radio-server` requires a rotation strategy for the pristine `./recordings` directory.

*   **Rationale:** The HQ Recorder process generates a bit-perfect 24-bit/48kHz FLAC copy of the stream. This consumes roughly 1.5 GB to 2 GB per hour. To prevent the local drive from filling up over continuous broadcast periods, the system must rely on an external rotation mechanism (e.g., a cron job moving old recordings to cold storage/NAS) rather than attempting to manage this within the Rust process itself.