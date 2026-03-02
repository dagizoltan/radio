# Prompt for Session 5: The AudioWorklet Web Component (The Player)

**Goal:** Implement the client-side `<radio-player>` Web Component, the Web Worker fetch loop, and the AudioWorklet for flawless browser audio playback.

**Context & Requirements:**
You are building the interactive frontend logic in `islands/player.ts` and the audio rendering thread in `islands/worklet.ts`.

**1. AudioWorklet (`worklet.ts`):**
- **Ring Buffer:** Implement a pre-allocated `Float32Array` ring buffer sized for ~20 seconds of audio (1,920,000 floats).
- **Zero-Allocation Demux:** In `process()`, iterate exactly 128 times (one per output frame):
  `outputs[0][0][i] = ringBuffer[(readPointer + i * 2) % length] * volume;`
  `outputs[0][1][i] = ringBuffer[(readPointer + i * 2 + 1) % length] * volume;`
- **Clock Drift Protection:** In the `port.onmessage` handler, check `chunk.length > freeSpace`. If the ring buffer is full (overflow), drop the incoming chunk entirely to resync clocks. Handle underruns natively by outputting silence.
- **State Commands:** Implement `"FLUSH"` to zero out `samplesAvailable` and `"QUERY_DEPTH"` to report the current buffer depth back to the main thread.

**2. Player Component (`player.ts`):**
- **Multi-Tab Prevention:** Use the Web Locks API (`navigator.locks.request("radio-player-singleton", ...)`). If the active tab closes, the background tab lock callback fires asynchronously; handle this by showing a "Playback transferred — click Play" UI rather than auto-starting (which breaks autoplay policy).
- **AudioContext Lifecycle:** Create the `AudioContext` and call `resume()` synchronously within the initial play button click handler. Wrap `audioCtx.audioWorklet.addModule` in a `try/catch` and gracefully handle module load failures.
- **iOS Interrupted State:** In the `onstatechange` handler, check `if (audioCtx.state === "suspended" || audioCtx.state === "interrupted")` and call `resume()` when the tab becomes visible.

**3. Web Worker Fetch Loop:**
- Move the core polling and chunk-fetching logic to a dedicated Web Worker to prevent background throttling.
- **Buffering Strategy:** Start playing at `Math.max(0, latest - 2)`. Dynamically track the segment download bandwidth (bytes per second). If the bandwidth drops close to the streaming rate (especially for VBR Opus), expand the buffer target (e.g., fetch 3-4 segments ahead).
- **Jump-Ahead Logic:** If `currentIndex < latest - 3`, snap to `latest - 1`. If `currentIndex` falls drastically behind, or a 404 occurs, instantly resync with the manifest.
- **Zero-Copy Buffer Pool:** When transferring decoded PCM from the WASM view to the Worklet, do NOT transfer the view's `.buffer` directly (it detaches WASM memory). Use a pre-allocated buffer pool: `pcmCopy.set(pcm); worklet.postMessage(pcmCopy, [pcmCopy.buffer]);`.
- **403 Refresh:** If a segment fetch returns `403 Forbidden`, hit `/api/token` to get a fresh HMAC token and retry.
- **Quality Switching:** Call `decoder.reset()` for HQ FLAC boundaries, but NOT for LQ Opus boundaries (gapless). Send `"FLUSH"` to the worklet and fetch the `next` index on quality change.

**Validation:**
Test the player extensively across browsers (Chrome, Firefox, iOS Safari). Verify that backgrounding the tab for 30 seconds, then foregrounding it, successfully triggers the `QUERY_DEPTH` -> `FLUSH` -> jump-to-live-edge sequence without playing 30 seconds of stale audio. Verify that toggling between HQ and LQ produces a clean audio break without pitch-shifting.
