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

*   **Rationale:** Producing verbatim subframes dramatically simplifies the encoder and decoder implementations. It eliminates the need for complex mathematical modeling and prediction algorithms in the hot path. While the files are larger than fully compressed FLAC, they remain perfectly compliant with the standard FLAC specification and retain the essential benefits: self-contained framing, robust sync codes, and checksums (CRC-8/CRC-16). Given the ThinkPad's upload bandwidth (~10.68 Mbps) comfortably exceeds the ~1.41 Mbps requirement for uncompressed CD audio, the size tradeoff is worthwhile for the massive gain in simplicity and reliability.

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

*   **Rationale:** Cloudflare R2 does not have a native, immediate Time-to-Live (TTL) feature that deletes objects precisely when they expire. To prevent unbounded storage growth for a continuous live stream, the server must manage the lifecycle manually. A rolling window ensures exactly 3 segments are stored at any time, capping storage at approximately 4.5 MB.
*   **Constraint:** Rolling window, not TTL. The uploader maintains a `VecDeque` of uploaded keys and deletes the oldest immediately when the window exceeds 3 segments.