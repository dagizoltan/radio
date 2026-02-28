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

The system builds its own segmented streaming protocol (a JSON manifest pointing to standalone 10-second FLAC/Opus files) rather than using industry standards like Icecast or HTTP Live Streaming (HLS).

*   **Rationale:**
    *   *Vs. Icecast:* Icecast requires a long-lived, dedicated TCP connection from every listener to a central server. This scales poorly and is vulnerable to transient network drops. Our architecture pushes static chunks to an edge CDN (Cloudflare R2), making delivery infinitely scalable, cacheable, and resilient to client network hiccups.
    *   *Vs. HLS/MPEG-DASH:* Modern platforms chunk media into segments (like we do) and use an `.m3u8` playlist. Browsers play these chunks natively using the Media Source Extensions (MSE) API. **However, browsers generally do not support FLAC via MSE.** If we used standard HLS, we would be forced to use lossy codecs (AAC/Opus) for everything, sacrificing our primary 24-bit lossless archival and broadcast goal. By building a custom chunk-fetcher, a WASM decoder, and an `AudioWorklet`, we completely bypass the browser's native codec limitations. The Deno server serves only the initial HTML shell and static assets. Both `manifest.json` and all audio segments are fetched directly from the R2/MinIO bucket by the browser, completely bypassing the Deno proxy for all media-related traffic.
    *   *Chunk Size vs. Latency:* While 10-second chunks inherently introduce a high latency of 10-20 seconds (the time to encode, upload, and for the client to fetch), the primary goal of this system is **production-grade, high-quality audio stability**, not ultra-low latency. Using 10-second segments significantly reduces the volume of HTTP PUT/GET requests, lowering overall system stress and making client-side playback alignment via WebAssembly far more reliable with fewer gapless playback transitions to compute.

## Why does the Recorder→Converter channel carry PCM, not FLAC frames?

- **Rationale:** An early design sent encoded FLAC frames over the raw channel, requiring the Converter to implement a FLAC decoder to recover PCM for normalisation. This added a full decode step in the audio hot path (potentially 1–2ms per period) and introduced a tight coupling between the encoder and converter implementations. Sending raw PCM is unambiguous, zero-copy-friendly (via `Arc<Vec<i32>>`), and requires no shared codec knowledge between the two tasks.
- **Archive encoding:** The Recorder Task independently encodes the same PCM buffer to FLAC for the archive write. This is a second encode of the same data, but it runs asynchronously and does not block the PCM send to the Converter. The two encodes are logically independent.
- **Constraint:** The `tokio::sync::mpsc` channel between Recorder and Converter carries `Arc<Vec<i32>>` (shared ownership, zero-copy clone). The Converter must not mutate the shared buffer — it clones it before passing to the normaliser (`normalizer.process(&mut buffer.to_vec())`).

## Why Opus for the LQ stream?

The Converter process encodes the lower-quality (LQ) fallback stream as Opus at 128 kbps stereo, wrapped in an Ogg container, using the pure-Rust `audiopus` crate for encoding and the `ogg` crate for container framing.

- **Rationale:** Opus is perceptually transparent at 128 kbps — indistinguishable from lossless for the vast majority of listeners and material. It achieves better compression than 16-bit FLAC at equivalent bitrates and far better than MP3, making it the optimal choice for a mobile-first fallback stream. Crucially, both `audiopus` (safe bindings to libopus, well-maintained) and a pure-Rust WASM Opus decoder (`opus-rs`) are available, keeping the build clean. The trade-off versus a second FLAC stream is the addition of a second WASM decoder module in the browser bundle (~60–80 KB gzipped), which is acceptable given the perceptual quality advantage.
- **No MP3:** There is no production-quality pure-Rust MP3 encoder available. Using `libmp3lame` via FFI would introduce a C dependency and cross-compilation complexity inconsistent with the project's architecture goals.
- **Ogg framing:** Ogg provides a self-synchronising packet structure ideal for streaming. An Ogg Opus stream can be resumed mid-stream by the decoder after a gap, which matters for the segment-based delivery model.
- **Bitrate mode:** The Opus encoder is configured for unconstrained VBR (`OPUS_SET_VBR(1)`, `OPUS_SET_VBR_CONSTRAINT(0)`). At a 128 kbps target, VBR segments vary in size: simple passages may produce ~100 KB, complex or loud passages ~200–220 KB, with an average around 160 KB over a typical broadcast hour. The bandwidth estimate and upload latency alarm thresholds account for this range. CBR would waste bandwidth on quiet passages and is not used.
- **Pre-skip gap:** The Opus spec requires each new encoder stream to output `pre_skip` samples of encoder lookahead before the first audio packet. At 48000 Hz with the default `pre_skip` of 312 samples (~6.5ms), each 10-second `.opus` segment introduces a ~6.5ms gap of silence at its start. The `OpusDecoder` discards these pre-skip samples as documented in `wasm-decoder.md`. Over 360 segments per hour, the cumulative gap in the LQ stream is approximately 2.3 seconds per hour of broadcast. This is inherent to segmented Opus streaming and is an accepted trade-off for the simplicity of self-contained segment files. The HQ FLAC stream has no equivalent gap.
- **Constraint:** LQ segment file extension is `.opus`. Quality is differentiated by path prefix (`live/hq/` vs `live/lq/`). The HQ FLAC decoder and LQ Opus decoder are separate WASM modules, each following the identical `push(bytes) → f32[]` API so the player fetch loop switches decoders by swapping a single reference.

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

*   **Rationale:** If a thousand listeners request the same 10-second chunk simultaneously, the edge CDN must serve it to prevent high egress costs and bucket throttling. LQ Opus segments average ~160 KB at 128 kbps VBR but range from ~100 KB to ~220 KB depending on programme content. Upload bandwidth estimates should use the upper bound (~220 KB) for worst-case planning.
*   **Constraint:** FLAC/Opus segments must be uploaded with `Cache-Control: public, max-age=31536000, immutable` (they never change once uploaded). The `manifest.json` must be uploaded with `Cache-Control: no-store, max-age=0` (it must always be fetched fresh to find the newest chunks) via the S3 `PUT` request's object metadata, not by the Deno proxy (which no longer handles the manifest at all).

## Clock Drift and Buffer Management

The browser client implements dynamic buffer management to counteract internal clock drift between the ThinkPad recording the audio and the listener's browser playing it.

*   **Rationale:** If the browser plays audio slightly faster than the server encodes 10-second chunks, the browser will eventually run out of data and stall. Conversely, if it plays slower, the buffer will bloat and latency will increase infinitely.
*   **Implementation:** The client monitors its internal buffer depth. If it detects drift accumulating beyond safe thresholds, it will either silently drop an old chunk (if drifting too far behind) or induce a brief pause to allow the buffer to refill (if playing too fast), ensuring long-term sync stability.

## CORS Security Policy

The cloud storage bucket (R2/MinIO) enforces a strict Cross-Origin Resource Sharing (CORS) policy.

*   **Rationale:** Because the `radio-client` Web Component fetches segments directly from the bucket (to bypass the Deno proxy and save bandwidth), the bucket must explicitly allow cross-origin `GET` requests from the Deno Deploy frontend URL, preventing unauthorized embedding or leeching from other domains.

## Archival Storage Rotation (Cold Storage)

The ThinkPad running the `radio-server` requires a rotation strategy for the pristine `./recordings` directory.

*   **Rationale:** The HQ Recorder process generates a bit-perfect 24-bit/48kHz FLAC copy of the stream. This consumes roughly 1.5 GB to 2 GB per hour. To prevent the local drive from filling up over continuous broadcast periods, the system must securely offload these archives.
*   **Implementation Strategy:** The system will use a secondary, separate S3-compatible bucket specifically for "cold storage" (e.g., a second Cloudflare R2 bucket in production, or a second MinIO bucket in local development). A daily automated cron job or systemd timer on the ThinkPad will upload the completed 24-hour recordings to this archive bucket and then delete them locally, entirely separating the live broadcast cloud infrastructure from the long-term archival storage.

## Observability and Telemetry

The architecture relies heavily on separate, independent processes working in tandem.

*   **Rationale:** Without centralized metrics, debugging a silent failure (e.g., the capture crate stalls, or R2 uploads begin failing) is incredibly difficult.
*   **Implementation Strategy:** The server must expose a minimal set of telemetry metrics (e.g., capture buffer overruns, normalizer LUFS targets, upload success/failure rates, rolling window size). These can be simple Prometheus-style `/metrics` endpoints pulled locally, or piped directly to the local operator UI. The Deno client should also report back basic playback stall metrics to an analytics endpoint to measure real-world listener experience.

## Graceful Degradation (Auto-Bitrate Switching)

The client must automatically handle degraded network conditions without manual user intervention.

*   **Rationale:** While the HQ FLAC stream is the priority, a mobile listener entering an area with poor signal will experience constant buffering. Expecting the user to manually click an "LQ Opus" button is a poor user experience.
*   **Implementation Strategy:** The Web Component's custom fetch loop must measure the download time of each 10-second chunk. If the fetch time consistently exceeds a safe threshold (e.g., it takes 8 seconds to download 10 seconds of audio), the client should automatically pivot to fetching from the LQ manifest. If network conditions improve and stabilize for several minutes, it can attempt an opportunistic pivot back to the HQ stream.
## Why 8-digit segment indices with wrap-at-100M?

- **Rationale:** The original 6-digit format (`{:06}`) supports 1,000,000 segments = ~115 days of continuous broadcast before index exhaustion. For a system intended to run continuously year-round, this is an operational boundary that would require manual intervention. Switching to 8-digit zero-padding (`{:08}`) extends the natural limit to ~31.7 years — beyond any realistic operational horizon — without changing the key format structure or S3 path semantics.
- **Wrap behaviour:** The index wraps at 100,000,000 using modular arithmetic. The client detects rollover via a sign-flip heuristic in the jump-ahead logic (see Data Flow doc). A true rollover after 31.7 years is operationally identical to a server restart.
- **Constraint:** All segment key formats use 8-digit zero-padding everywhere: server `PUT` requests, manifest `latest` field values, and client URL construction.

## Why Web Locks API for multi-tab prevention?

- **Rationale:** Opening the stream in two browser tabs simultaneously doubles R2 egress from a single user, doubles WASM decoder CPU load, and can produce audio bleed between tabs on shared audio devices. The Web Locks API (`navigator.locks.request`) provides a browser-native, cross-tab mutex. If the lock cannot be acquired (another tab holds it), the player shows a "Stream is already playing in another tab" message and disables the play button rather than starting a second decoder pipeline.
- **Fallback:** On browsers without Web Locks support (pre-Chromium Edge, some older Safari versions), the lock check is skipped and multi-tab is permitted silently. The player logs a console warning.
- **Constraint:** The lock name is `"radio-player-singleton"` and is held for the lifetime of the playback session. It is released automatically by the browser when the tab is closed or navigated away.

## Why explicit AudioContext resume on tab visibility change?

- **Rationale:** Some browsers (particularly mobile Safari and Chrome on Android) automatically suspend the `AudioContext` when a tab is hidden or the screen is locked. The fetch loop runs in a dedicated Web Worker and is unaffected by tab visibility, so audio chunks continue arriving and filling the ring buffer. When the user returns to the tab, the ring buffer may be full of stale audio. Without explicit context management, playback resumes from the buffered (stale) position rather than the live edge.
- **Implementation:** The player registers a `document.addEventListener("visibilitychange", ...)` handler. On `"visible"`, it calls `audioCtx.resume()` and, if the ring buffer contains more than 1.5 segments worth of data, sends a `"FLUSH"` to the worklet and jumps the fetch index to `latest - 1` to re-anchor to the live edge. This ensures returning from a background tab feels instantaneous and live rather than delayed by buffered stale audio.
