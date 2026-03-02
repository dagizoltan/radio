# Prompt for Session 5: The AudioWorklet Web Component (The Player)

**Goal:** Implement the client-side `<radio-player>` Web Component, the Web Worker fetch loop, and the AudioWorklet for flawless browser audio playback.

**Context & Requirements:**
You are building the interactive frontend logic in `islands/player.js` and the audio rendering thread in `islands/worklet.js`. Ensure you are writing standard ES modules (pure JavaScript, NO TypeScript).

**1. AudioWorklet (`worklet.js`):**
- **Ring Buffer:** Implement a pre-allocated `Float32Array` ring buffer sized for ~20 seconds of audio (1,920,000 floats). Track `readPointer`, `writePointer`, and `samplesAvailable`.
- **Zero-Allocation Demux:** In `process(inputs, outputs, parameters)`, output silence if `samplesAvailable < 256` (underrun). Otherwise, iterate `i` from 0 to 127:
  ```javascript
  outputs[0][0][i] = ringBuffer[(readPointer + i * 2) % 1920000] * volume;
  outputs[0][1][i] = ringBuffer[(readPointer + i * 2 + 1) % 1920000] * volume;
  ```
  Then advance `readPointer = (readPointer + 256) % 1920000` and decrement `samplesAvailable`.
- **Clock Drift Protection:** In `port.onmessage`, calculate `freeSpace = 1920000 - samplesAvailable`. If `chunk.length > freeSpace` (overflow), drop the chunk and post an `"OVERFLOW"` warning to the main thread.
- **State Commands:** Handle `"FLUSH"` (zero `samplesAvailable`, align pointers) and `"QUERY_DEPTH"` (post `samplesAvailable` back).

**2. Player Component (`player.js`):**
- **Multi-Tab Prevention:**
  ```javascript
  navigator.locks.request("radio-player-singleton", async (lock) => {
    // Hide "in use" warning, enable play button.
    await new Promise(r => this._releaseLock = r); // Hold indefinitely
  });
  ```
  If a background tab gets the lock later, do NOT auto-play. Update UI to "Playback transferred — Click Play".
- **AudioContext Lifecycle:** Create the `AudioContext` and call `resume()` *synchronously* in the play button click handler before any `await`. Then wrap `await audioCtx.audioWorklet.addModule('/static/worklet.js')` in a `try/catch`.
- **iOS Interrupted State:** In `audioCtx.onstatechange`, check `if (audioCtx.state === "suspended" || audioCtx.state === "interrupted")` and if document is visible, `audioCtx.resume()`.

**3. Web Worker Fetch Loop (`fetch_worker.js`):**
- **Architecture:** Move the `setInterval` fetch loop into a standard Web Worker to survive backgrounding. Use `postMessage` to communicate with `player.js`.
- **Dynamic Buffering Strategy:**
  - Track `bytes_downloaded` and `time_taken_ms` per segment.
  - Calculate `bandwidth_bps`. If `bandwidth_bps` drops near the stream bitrate (e.g., `< 1,500,000 bps` for HQ FLAC or `< 800,000 bps` for LQ FLAC), dynamically increase the pre-roll from `latest - 2` to `latest - 4`.
- **Jump-Ahead Logic:** If `currentIndex < latest - max_buffer_target - 1`, snap `currentIndex` to `latest - 1`. If `404`, fetch manifest again instantly.
- **Zero-Copy Buffer Pool:** Receive WASM memory view. Pull from a recycled buffer array:
  ```javascript
  const pcmCopy = pool.pop() || new Float32Array(pcm.length);
  pcmCopy.set(pcm);
  workletPort.postMessage(pcmCopy, [pcmCopy.buffer]); // Transfer
  ```
- **403 Refresh:** On 403, `await fetch('/api/token', { method: 'POST' })`, update the token, and retry the segment.
- **Quality Switching:** Call `decoder.reset()` for BOTH streams (since both are standard standalone FLAC files now). Send `"FLUSH"` to worklet, increment `currentIndex` and fetch the next segment immediately.

## 4. Testing Contract
- **Ring Buffer Pointer Math:** Write a JS unit test for the `RadioProcessor` class. Manually `handleMessage` a chunk of 1,000,000 floats. Then `handleMessage` another chunk of 1,000,000 floats. Assert that `samplesAvailable` correctly caps and rejects the overflow, and that `writePointer` accurately wraps around the 1,920,000 array length using modulo arithmetic without throwing an out-of-bounds exception.

## 5. Error Recovery Matrix
- **Worker 404 Infinite Loop:** If a requested segment returns a 404, the worker MUST sleep for at least `segment_s / 2` before polling the manifest again. Do not spin-loop 404s, or the client will self-DDoS the CDN.
- **AudioContext Autoplay Blocked:** If the user hasn't interacted with the DOM, `audioCtx.state` will remain `"suspended"` after creation. The UI must detect this state and display a "Click anywhere to unlock audio" overlay.
