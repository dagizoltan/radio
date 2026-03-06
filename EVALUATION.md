# System Evaluation Report

This report outlines bugs, issues, and areas for enhancement identified during a deep architectural and codebase review of the Lossless Radio Player system (Rust Server + JS/WASM Client).

## Part 1: Bugs and Issues

### Server (Rust)
1. **Unbounded Allocation in `ConverterTask` (`converter.rs`)**:
   - `hq_accumulator` and `lq_accumulator` are populated iteratively but never cleared completely if `frame_counter` does not cleanly align with the 10.24-second boundary. Wait, `hq_accumulator.clear()` is called after reaching the threshold, but the condition `if self.frame_counter >= 491_520` might occasionally overshoot by a small margin depending on `pcm_arc` lengths, leaving frames behind if it doesn't align cleanly. Wait, `frame_counter` is incremented by exactly 4096 frames each loop, and 491,520 % 4096 == 0. However, if the server shuts down or drops frames, any remaining frames are flushed, but `self.hq_accumulator` is copied entirely, which is correct.
   - However, a real bug exists here: **LQ decimation logic (`converter.rs:59`) averages samples but fails to sign-extend correctly for negative 24-bit values**. The values coming from `CaptureLoop` are properly sign-extended to 32-bit (`i32`), but doing `l1 / 2` and `l2 / 2` directly averages them. Then they are shifted `>> 8` to convert from 24-bit to 16-bit. `l_avg >> 8` on a negative `i32` performs an arithmetic right shift, which preserves the sign. But the bitwise truncation to 16-bits later might cause issues if not clamped properly.
2. **Missing Anti-Aliasing Filter Before Decimation**:
   - In `ConverterTask` (`converter.rs`), LQ downsampling is performed by simple averaging of adjacent samples (`(l1/2) + (l2/2)`). This acts as a very weak low-pass filter (a 2-tap boxcar filter) but does not properly filter out frequencies between 12 kHz and 24 kHz (the Nyquist frequency of the new 24kHz sample rate). This will cause severe aliasing artifacts in the LQ stream.
3. **FlacEncoder `encode_frame` Block Size Bug (`flac.rs`)**:
   - The method uses `interleaved.len()` which might contain varying elements if the input array does not precisely match `self.block_size * channels`. It blindly iterates `0..self.block_size` inside `encode_frame`, which could cause a panic (`index out of bounds`) if `interleaved` slice length is smaller than `self.block_size * self.channels`.
4. **Incorrect S3 Put Retry Logic (`uploader.rs`)**:
   - In `upload_with_retry`, the `put_s3` function is called repeatedly on failure. However, `payload_hash` in `put_s3` uses `hex::encode(Sha256::digest(&body))`. If the request fails due to network issues, it retries up to 3 times. But wait, if `put_s3` fails due to 503, it retries. If it fails due to 403 `RequestTimeTooSkewed`, it detects NTP clock drift but still retries without updating the time, which is guaranteed to fail again.
5. **Memory Leak / Unbounded `window` Queue (`uploader.rs`)**:
   - In `UploaderTask`, `self.window.push_back(index)` is used to track segments to delete. However, it only pops and deletes when `self.window.len() > 10`. If S3 deletion fails, the index is still popped and lost, meaning it will never be retried for deletion, leaking objects in S3.
6. **Inefficient `RecorderTask` Channel Routing (`recorder.rs`)**:
   - If `selected_channel` is "left" or "right", the code copies samples in place (`pcm_data[i+1] = pcm_data[i]`). If `should_stream` is false, it skips broadcasting. However, it modifies the data passed to the local archiver, meaning the local archive is ALSO converted to mono, which might not be the intended behavior (usually archives are kept pristine).
7. **Thread Blocking in `UploaderTask` Background Cleanup (`uploader.rs`)**:
   - `Self::background_cleanup` is spawned as a detached Tokio task. It uses `reqwest::Client` to list and delete objects. This is correct, but the S3 List API pagination is entirely ignored. If there are thousands of old objects (e.g., after the server was down for a while), `doc.descendants()` will only parse the first 1000 objects. Oldest objects will not be cleaned up if they fall outside the first page.
8. **Hardcoded S3 Bucket Endpoint Logic**:
   - In `generate_sigv4`, `host` is extracted using `.replace("https://", "").replace("http://", "").split('/').next()`. If the endpoint contains a port (e.g., `http://minio:9000`), the port is included in the `Host` header. This is correct for some S3-compatible APIs but can cause issues with AWS S3 if virtual-host style addressing is expected.
9. **Metrics Data Race / SSE Lag (`main.rs`)**:
   - The `waveform` metrics are locked and cloned every 100ms in a background task. The `CaptureLoop` modifies `waveform` in `recorder.rs` without debouncing. If the `CaptureLoop` hangs (e.g., waiting for ALSA), the SSE will continue to emit the *same* frozen waveform 10 times a second.
10. **File Handle Leak on Rotation (`recorder.rs`)**:
    - When rotating the archive file, `archive_file.take()` is called, and `file.sync_all()` is executed. However, `File` is dropped immediately after. While Rust closes the file on drop, any I/O errors during `sync_all()` or `drop` are silently ignored.
11. **ALSA Unrecoverable State (`capture.rs`)**:
    - If `SNDRV_PCM_IOCTL_READI_FRAMES` returns an error other than `EPIPE`, `EAGAIN`, `EWOULDBLOCK`, or `ENODEV`, it returns an error `Err(err)`. `RecorderTask` sees this, logs "ALSA read err", sleeps 1s, and continues with the same `capture_loop` instance, which is likely permanently poisoned, resulting in an infinite 1-second error loop.
12. **Missing `Keep-Alive` in Fetching S3 (`fetch_worker.js` / Server)**:
    - S3/R2 endpoints typically close idle connections. `reqwest` connection pooling might hold onto dead connections if not configured with `pool_idle_timeout`.

### Client (JS/WASM)
13. **AudioWorklet Overflow Drop Strategy (`worklet.js`)**:
    - If a chunk is larger than the remaining `freeSpace`, the `RadioProcessor` completely drops the chunk (`return; // Drop chunk`) and posts an `OVERFLOW` message. This results in an audible gap. It should instead write as much of the chunk as possible to fill the buffer, or expand the ring buffer dynamically.
14. **AudioContext Suspension Race Condition (`player.js`)**:
    - `navigator.locks.request` is used to ensure a singleton player. However, the `AudioContext` is created in `togglePlay()`. If a user clicks play before the lock is resolved, it could trigger an unhandled state. Moreover, Chrome requires `AudioContext` to be created/resumed on a user gesture.
15. **Unbounded Float32Array Memory Pool (`fetch_worker.js`)**:
    - The zero-copy pool array (`const pool = []`) receives buffers back from the AudioWorklet. However, if the network bandwidth is much faster than playback (which it always is), `fetchNextSegment()` runs on a `setTimeout` based on segment length, which controls the ingestion rate. However, if `bufferTarget` changes dynamically, it can enqueue multiple chunks. The `pool` can grow unconditionally if there is a mismatch in chunk sizes or timing.
16. **WASM Memory Leak (`decoder.js`)**:
    - `decoder.decode(bytes)` returns `new Float32Array(len)` and copies data from `this.wasmMemory.buffer`. However, there is no corresponding `free` or `drop` called on the WASM side to release the memory allocated for the decoded PCM frame inside the Rust FLAC decoder. This will cause the WASM memory to grow indefinitely until the browser tab crashes (OOM).
17. **Manifest Token Refresh Loop (`main.js` / `fetch_worker.js`)**:
    - In `main.js`, `generateToken` checks the origin and `cf-connecting-ip`. The generated token is valid for the current hour (`Math.floor(Date.now() / 3600000)`). If the client fetches a segment exactly on the hour boundary, it will get a 403, call `refreshToken()`, and retry. However, if the server and client clocks are skewed, they could enter an infinite 403 -> refresh -> 403 loop.
18. **Stale Manifest Flag Never Clears**:
    - In `fetch_worker.js`, if the manifest is detected as stale (`Date.now() - manifest.updated_at > segmentLengthSec * 3 * 1000`), it posts `STALE_MANIFEST: true`. But it does not pause playback or attempt to switch to a fallback URL; it just updates UI text. If it becomes unstale, the UI does not automatically recover the "Streaming Lossless" text unless the player is paused and played again.

### Docker / Architecture
19. **Docker Bind Mount Permissions (`docker-compose.yml`)**:
    - `radio-server` runs as `root` (default) inside the container and bind-mounts `/dev/snd` and `./archive`. Files created in `./archive` will be owned by `root`, making them difficult to manage or delete from the host machine without `sudo`.
20. **Lack of NTP Synchronization**:
    - The whole system (S3 uploads, AWS SigV4, Token generation) heavily relies on accurate wall-clock time. Neither the `Dockerfile` nor `docker-compose.yml` ensures NTP sync or passes time synchronization into the containers, leading to inevitable `RequestTimeTooSkewed` AWS errors or Token hour mismatches.

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
8. **Paginated S3 Cleanup (Server)**:
   - Update `background_cleanup` in `uploader.rs` to parse the `NextContinuationToken` from the S3 XML response and loop until all old segments are properly paginated and deleted.
9. **Use `SharedArrayBuffer` for Worklet (Client)**:
   - Instead of posting messages back and forth and using a custom `pool`, utilize a `SharedArrayBuffer` for the ring buffer. `fetch_worker.js` can write directly into the shared memory space, and the `AudioWorklet` can read from it using `Atomics`, achieving true zero-copy and eliminating garbage collection pauses on the main/worker threads.
10. **Implement Healthcheck Endpoints (Server)**:
    - Add a `/healthz` endpoint to the Axum router that checks the ALSA device state, Minio connection, and internal channel capacities. Expose this in `docker-compose.yml` using the `healthcheck` directive to automatically restart the container if the audio pipeline stalls.
11. **Client-Side WASM Pre-allocation (Client)**:
    - Instead of re-allocating a new `Float32Array` on the WASM heap for every decoded frame, allocate a single output buffer during `LosslessDecoder.init()`. The Rust decoder should write into this fixed memory region, and Javascript can slice it, preventing fragmentation of the WASM heap over days of continuous listening.