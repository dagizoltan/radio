# System Evaluation Report

This report outlines bugs, issues, and areas for enhancement identified during a deep architectural and codebase review of the Lossless Radio Player system (Rust Server + JS/WASM Client).

## Part 1: Bugs and Issues

*(All previously identified bugs have been successfully fixed and removed from this list as requested).*

---

## Part 2: Enhancements and Optimizations

1. **Implement Proper Resampling (Server)**:
   - Replace the simplistic sample averaging in `converter.rs` with a proper polyphase FIR filter or a fast windowed-sinc interpolator to prevent severe aliasing in the LQ stream. This will drastically improve the audio quality of the low-bandwidth stream.
2. **Dynamic FLAC Block Sizes (Server)**:
   - The current FLAC encoder uses a fixed block size (4096 or 2048). FLAC achieves significantly better compression ratios (and thus saves R2 bandwidth) by dynamically adjusting block sizes based on signal transients. Integrating a robust FLAC library (like `flac-sys` or a pure Rust alternative with LPC and dynamic blocking) would save up to 20% bandwidth.
3. **Use WebRTC for Low-Latency Live Streaming (Architecture)**:
   - Currently, the system uses HTTP chunked segments (HLS-like) with 10.24s segments, resulting in a mandatory 20-30 second latency (pre-roll + segment duration). Moving to WebRTC (via something like Mediasoup or Pion) would reduce latency to < 500ms, while keeping the FLAC archive loop for VOD.
4. **Adaptive Bitrate Switching Improvements (Client)**:
   - The `bandwidthEma` calculation in `fetch_worker.js` only measures the download speed of the FLAC chunks. FLAC is VBR (Variable Bitrate). A highly compressible chunk (e.g., silence) downloads very fast, artificially inflating the `bandwidthEma` calculation, potentially causing the player to switch to HQ right before a complex, high-bitrate song drops, causing buffering. Switching logic should divide by the *uncompressed* duration or track buffer health (samples available in worklet) rather than just fetch time.
5. **Memory-Mapped Files for Archiving (Server)**:
   - Instead of continuously calling `File::create` and `write_all` in `recorder.rs`, use memory-mapped files (`mmap`) for the rolling archive. This ensures that in the event of a catastrophic power failure, the kernel has already paged the captured audio to disk, reducing data loss.
6. **Graceful Degradation for S3 Uploads (Server)**:
   - If Minio/R2 is temporarily down, the `UploaderTask` will simply drop segments and print an error. It should instead spill over to a local disk buffer (SQLite or RocksDB) and background-sync them when the connection is restored, ensuring zero packet loss during cloud outages.
7. **AudioWorklet Interpolation (Client)**:
   - The `RadioProcessor` currently drops chunks on overflow or returns silence on underrun. To hide minor network jitters (e.g., a 50ms gap), implement a slight time-stretching (WSOLA) or fade-in/fade-out interpolation algorithm during underruns instead of harsh digital silence clicks.
8. **Use `SharedArrayBuffer` for Worklet (Client)**:
   - Instead of posting messages back and forth and using a custom `pool`, utilize a `SharedArrayBuffer` for the ring buffer. `fetch_worker.js` can write directly into the shared memory space, and the `AudioWorklet` can read from it using `Atomics`, achieving true zero-copy and eliminating garbage collection pauses on the main/worker threads.
9. **Implement Healthcheck Endpoints (Server)**:
    - Add a `/healthz` endpoint to the Axum router that checks the ALSA device state, Minio connection, and internal channel capacities. Expose this in `docker-compose.yml` using the `healthcheck` directive to automatically restart the container if the audio pipeline stalls.
10. **Client-Side WASM Pre-allocation (Client)**:
    - Instead of re-allocating a new `Float32Array` on the WASM heap for every decoded frame, allocate a single output buffer during `LosslessDecoder.init()`. The Rust decoder should write into this fixed memory region, and Javascript can slice it, preventing fragmentation of the WASM heap over days of continuous listening.