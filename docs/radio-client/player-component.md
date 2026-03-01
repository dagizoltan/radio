# Player Web Component (islands/player.ts)

The `islands/player.ts` file defines the interactive client-side logic for the `<radio-player>` Web Component.

## Initialization

The component is defined using `customElements.define("radio-player", class extends HTMLElement { ... })`. It does not use a frontend framework.

### connectedCallback

When the element is inserted into the DOM, `connectedCallback` fires:

1.  **Render:** It populates `this.innerHTML` with the player's UI (if not already hydrated by SSR):
    *   A `<canvas>` element for the waveform.
    *   A metadata section displaying the title and a Quality Selector toggle (e.g., `<select>` or buttons for `HQ` / `LQ`). HQ displays "Hi-Res · 48kHz · 24-bit · FLAC (Lossless)", LQ displays "Standard · 48kHz · Opus · 128kbps".
    *   A controls row containing a play/stop button, an animated live indicator dot, a latency display, and a volume `<input type="range">`.
2.  **Event Binding:** It attaches click listeners to the play/stop button, input listeners to the volume slider, and change listeners to the Quality Selector.

## Multi-Tab Prevention

To prevent duplicate decoder pipelines and doubled R2 egress from a single user, the player uses the **Web Locks API** to enforce a single active playback instance across all tabs:

> ⚠️ **The following pattern is shown for illustration only and will silently fail — see the implementation note below for the correct approach.**

```javascript
async connectedCallback() {
  // ... render UI ...

  this._lockController = new AbortController();

  const acquired = await new Promise(resolve => {
    navigator.locks.request(
      "radio-player-singleton",
      { ifAvailable: true, signal: this._lockController.signal },
      async (lock) => {
        if (!lock) { resolve(false); return; }
        resolve(true);
        // Lock is held until this promise resolves (i.e. until playback stops)
        await new Promise(r => (this._releaseLock = r));
      }
    ).catch(() => resolve(false));
  });

  if (!acquired) {
    this._showMessage("Stream is already playing in another tab.");
    this._disablePlayButton();
    return;
  }
}

stopPlayback() {
  this._releaseLock?.();       // Release the Web Lock
  this._lockController.abort();
  // ... stop AudioContext, fetch worker, etc.
}
```

**Implementation note — async `connectedCallback`:** Custom element lifecycle callbacks must be synchronous — browsers do not `await` an async `connectedCallback`. If `connectedCallback` is declared `async`, the function returns a `Promise` immediately and the `await` inside runs in the background. This means code after the lock check (rendering, event binding) will execute *before* the lock resolves if written naively. Structure the implementation so the UI renders synchronously in `connectedCallback`, with the play button initially disabled, and enable it only after the async lock resolves:

```javascript
connectedCallback() {
  this._renderUI();          // synchronous — renders UI, play button disabled
  this._acquireLock();       // async — enables play button only after lock acquired
}

async _acquireLock() {
  if (!("locks" in navigator)) {
    this._enablePlayButton();
    return;
  }

  // Notice we omit `ifAvailable: true`. The background tab will wait indefinitely
  // for the active tab to release the lock, at which point it will enable its own play button.
  navigator.locks.request("radio-player-singleton", async (lock) => {
    this._hideMessage();
    this._enablePlayButton();
    // Hold the lock until this playback session ends
    await new Promise(r => (this._releaseLock = r));
  }).catch((err) => {
    console.error("Lock error", err);
  });

  this._showMessage("Stream is already playing in another tab.");
}
```

**Fallback:** Check `"locks" in navigator` before using the API. On unsupported browsers, skip the lock and log a console warning. Multi-tab is then silently permitted on those browsers.

**Tab close / navigation:** The browser automatically releases the lock when the tab is closed or navigated away, so the lock is never orphaned.

## Playback Sequence

When the user clicks the "Play" button, the following sequence occurs:

1.  **AudioContext:** Create a new `AudioContext` with `{ sampleRate: 48000 }`.
2.  **Worklet Loading:** Await `audioCtx.audioWorklet.addModule("/static/worklet.js")`.
    **Ring Buffer Sizing:** The AudioWorklet ring buffer must hold at least 2 full 10-second segments: `48000 × 2 channels × 20 seconds = 1,920,000 floats` (~7.3 MB). This prevents underrun when a segment takes close to its full 10-second window to download on a degraded connection.
3.  **Node Creation:** Create an `AudioWorkletNode` named `"radio-processor"`. Pass the initial volume from the slider as a parameter.
4.  **Analyser Chain:** Create an `AnalyserNode` for the waveform visualizer. Chain them: `workletNode.connect(analyserNode).connect(audioCtx.destination)`.
5.  **MediaSession Integration:** Register the stream metadata with the OS-level media controls (e.g., lock screen, keyboard play/pause keys) using `navigator.mediaSession.metadata = new MediaMetadata({ title: "Lossless Vinyl Radio", artist: "Live Stream" });` and set action handlers (`setActionHandler('play', ...)`).
6.  **WASM Import:** Dynamically import the WASM decoder module:
    ```javascript
    // Import both decoders once during initialisation
    import initFlac, { FlacDecoder } from "/static/flac_decoder.js";
    import initOpus, { OpusDecoder } from "/static/opus_decoder.js";
    await Promise.all([initFlac(), initOpus()]);

    // Active decoder reference, swapped on quality change
    const flacDecoder = new FlacDecoder();
    const opusDecoder = new OpusDecoder();
    let decoder = currentQuality === 'hq' ? flacDecoder : opusDecoder;

    // On quality switch: call `decoder.reset()`, send `"FLUSH"` to the worklet, then swap the `decoder` reference, and advance to the NEXT segment index to avoid audible rewind.
    ```
7.  **Fetch Loop (`setInterval` / Web Worker):** Start the main fetch loop. Crucially, the fetch polling loop must **not** rely on `requestAnimationFrame` or `setTimeout` running on the main thread, because browsers heavily throttle (or pause entirely) background tabs. To keep audio playing smoothly while the listener browses other tabs, the fetch loop should be driven by a `setInterval` running in a dedicated Web Worker, which passes fetch commands or chunk events back to the main thread `MessagePort`.
8.  **Waveform Animation Loop:** The visual waveform updates independently using `requestAnimationFrame`. When the tab is backgrounded, it correctly pauses rendering to save battery, but the separate Web Worker continues fetching audio chunks.

## Fetch Loop Algorithm

The core of the player is the fetch loop, which continuously polls for new segments and streams them to the decoder.

1.  **Manifest Poll:** Fetch the manifest from the Deno SSR endpoint (e.g., `/api/manifest`), which proxies it from R2 and applies edge caching. This avoids massive Class B GET costs on Cloudflare R2 by coalescing client requests.
    *   **Optimization:** Rely on the `Cache-Control: s-maxage=5` header set by the Deno server to utilize CDN edge caching.
    *   If offline, update the UI and retry after a delay.
    *   If live, extract `latest` segment index and `segment_s` duration.
2.  **Buffering Strategy:** Start playing 2 segments behind the `latest` index to build a small buffer against network jitter.
3.  **Jump-Ahead Logic:**
    *   If the player's current segment index is ahead of `latest`, sleep for `segment_s / 2` and repoll.
    *   If the player falls more than 3 segments behind `latest` (e.g., due to pausing or network stall), immediately jump to `latest - 1`.
    *   **Server Restart Detection:** If the `latest` index abruptly drops by a large amount or resets to `0` (and it is not a normal rollover from 99,999,999), this indicates the backend server restarted. The continuous Opus stream state has been broken. In this case, call `decoder.reset()` before fetching the new segment to avoid decoding errors or audio glitches.
4.  **Segment Streaming (Direct to CDN):**
    *   Construct the correct URL path using the `R2_PUBLIC_URL` base injected by the server.
    *   Construct the segment URL based on quality and append the security token if present:
    ```javascript
    const ext = currentQuality === 'hq' ? 'flac' : 'opus';
    const padded = String(currentIndex).padStart(8, '0');
    let url = `${this.dataset.r2Url}/live/${currentQuality}/segment-${padded}.${ext}`;
    if (this.dataset.token) url += `?token=${this.dataset.token}`;
    ```
    HQ segments are FLAC (`.flac`). LQ segments are raw continuous Opus packets (`.opus`). Quality is differentiated by path prefix and file extension.
    *   Get a `ReadableStreamDefaultReader` from the response body.
    *   Loop `reader.read()`. As each `Uint8Array` chunk arrives:
        *   Pass the chunk to the WASM decoder: `const pcm = decoder.push(chunk)`.
        *   If `pcm` (a `Float32Array`) has length > 0, transfer it to the worklet to avoid main-thread garbage collection. **CRITICAL:** Do NOT transfer `pcm.buffer` directly, as it points to the WASM instance's linear memory. Transferring it detaches the memory buffer, instantly crashing the WASM decoder. Instead, you **must** implement a buffer pool to copy the data into and transfer to the worklet. Creating a `new Float32Array(pcm)` on every chunk will eventually trigger main-thread garbage collection pauses. Use a `MessageChannel` (or simply `port.onmessage`) where the Worklet returns empty buffers for the decoder to reuse, achieving true zero-allocation playback:
        ```javascript
        // Assume `pool` is an array of recycled Float32Arrays
        const pcmCopy = pool.pop() || new Float32Array(pcm.length);
        pcmCopy.set(pcm); // Copy out of WASM bounds into pooled buffer
        workletNode.port.postMessage(pcmCopy, [pcmCopy.buffer]); // Transfer buffer
        ```
5.  **Quality Switching:** If the user changes the quality mid-stream:
    *   The current `reader.cancel()` is called.
    *   A `"FLUSH"` message is sent to the `AudioWorklet` via `postMessage` to instantly clear any buffered PCM data. This prevents an audible pitch-shift or pop when the new codec chunks arrive.
    *   The `currentQuality` state updates.
    *   The fetch loop immediately attempts to fetch the *next* `currentIndex` (`currentIndex + 1`) using the new quality path (`hq` FLAC or `lq` Opus).
    **Audio forward skip on quality switch:** Because the fetch loop fetches the *next* `currentIndex` from byte 0 in the new codec, the listener may experience a slight forward skip (up to 10 seconds of audio) after a quality switch (the portion of the current segment skipped). This produces a clean decode boundary with no codec state bleed and provides a better user experience than re-playing up to 10 seconds of already-heard audio.
6.  **Iteration:** When `reader.read()` returns `done: true` normally, increment the `currentIndex`.
7.  **Latency Display:** Calculate and update the UI with the estimated latency: `(latest - currentIndex) * segment_s` seconds behind live.
## AudioContext Lifecycle and Background Tab Handling

### Autoplay Policy
Browsers require a user gesture before an `AudioContext` can enter the `"running"` state. The `AudioContext` must be created **and** `audioCtx.resume()` must be called synchronously within the same click event handler as the user's Play button tap. Do not `await` any asynchronous operations (worklet loading, WASM init) before calling `resume()` — on iOS Safari, crossing a task boundary between the gesture and `resume()` breaks the autoplay unlock.

Correct sequence:
```javascript
playButton.addEventListener("click", async () => {
  // Step 1: Create context and call resume() synchronously in gesture handler
  const audioCtx = new AudioContext({ sampleRate: 48000 });
  audioCtx.resume(); // Do NOT await — must be sync within gesture

  // Step 2: Now await async setup
  await audioCtx.audioWorklet.addModule("/static/worklet.js");
  // ... rest of initialisation
});
```

### Suspension on Tab Hide (Background Tab Audio Continuity)
The fetch loop runs in a dedicated **Web Worker** (not `requestAnimationFrame` or main-thread `setInterval`), so audio chunks continue arriving when the tab is hidden. However, some browsers (mobile Safari, Chrome on Android) automatically suspend the `AudioContext` when the tab is hidden, causing the ring buffer to fill with unconsumed audio.

Register a visibility change handler to manage this:
```javascript
document.addEventListener("visibilitychange", async () => {
  if (document.visibilityState === "visible") {
    // Resume context if suspended
    if (audioCtx.state === "suspended") await audioCtx.resume();

    // If ring buffer has accumulated > 1.5 segments of stale audio, flush and re-anchor
    // NOTE: postMessage returns undefined. Do NOT try to use a return value here.
    // The DEPTH_RESPONSE message arrives asynchronously in workletNode.port.onmessage.
    workletNode.port.postMessage({ type: "QUERY_DEPTH" });
  }
});

// In worklet port message handler:
workletNode.port.onmessage = (e) => {
  if (e.data.type === "DEPTH_RESPONSE" && e.data.samplesAvailable > 48000 * 15) {
    // More than 15 seconds buffered — stale; flush and jump to live edge
    workletNode.port.postMessage("FLUSH");
    fetchWorker.postMessage({ type: "JUMP_TO_LATEST" });
  }
};
```

### AudioContext State Change Handler
```javascript
audioCtx.onstatechange = () => {
  if (audioCtx.state === "suspended" && document.visibilityState === "visible") {
    // Context was suspended unexpectedly while tab is visible (e.g. OS audio focus lost)
    audioCtx.resume();
  }
};
```

### MediaSession Integration
Register MediaSession action handlers to integrate with OS-level media controls (lock screen, headphone buttons, Bluetooth):
```javascript
navigator.mediaSession.metadata = new MediaMetadata({
  title: "Lossless Vinyl Radio",
  artist: "Live Stream"
});
navigator.mediaSession.setActionHandler("play", () => {
  audioCtx.resume();
  fetchWorker.postMessage({ type: "RESUME" });
});
navigator.mediaSession.setActionHandler("pause", () => {
  audioCtx.suspend(); // Pauses audio output; fetch worker continues buffering
  fetchWorker.postMessage({ type: "PAUSE" });
});
```
Note: Calling `audioCtx.suspend()` from the MediaSession `pause` handler pauses audio output cleanly. The fetch worker should continue fetching during a MediaSession pause to avoid re-buffering delay on resume. The ring buffer absorbs the incoming audio silently while the context is suspended.

**Stale buffer on MediaSession resume:**

When the user resumes via a MediaSession action (e.g., lock screen play button or headphone control), the ring buffer may contain audio that was buffered during the pause. Unlike the `visibilitychange` path, this is an explicit user-initiated resume — the expectation is immediate live audio, not continuation from the pause point.

```javascript
navigator.mediaSession.setActionHandler("play", () => {
  audioCtx.resume();
  fetchWorker.postMessage({ type: "RESUME" });

  // Check buffer depth; flush stale audio if the pause was long
  workletNode.port.postMessage({ type: "QUERY_DEPTH" });
  // Response handled in workletNode.port.onmessage (DEPTH_RESPONSE)
  // Same flush + jump-to-latest logic as the visibilitychange handler
});
```

The `DEPTH_RESPONSE` handler in `workletNode.port.onmessage` is already defined for the `visibilitychange` case. It applies identically here: if `samplesAvailable > 48000 * 15` (more than 15 seconds buffered), send `"FLUSH"` to the worklet and post `{ type: "JUMP_TO_LATEST" }` to the fetch worker.

**Threshold rationale:** 15 seconds corresponds to roughly 1.5 segments. Any pause long enough to buffer more than 1.5 segments means the listener would resume noticeably behind the live edge. Below this threshold (e.g., a 5-second pause), resuming from the buffer is preferable to discarding buffered audio and incurring a re-buffer delay.

### Cross-Platform Compatibility

| Platform | Known Issue | Mitigation |
|---|---|---|
| All platforms | MediaSession resume after long pause plays stale buffered audio | `QUERY_DEPTH` on every MediaSession play action; flush + jump if > 15s buffered |
| iOS Safari 14.5+ | `audioCtx.resume()` must be synchronous within gesture handler | Call `resume()` before any `await` in the click handler (see above) |
| iOS Safari < 14.5 | `AudioWorklet` not supported | Detect with `"audioWorklet" in AudioContext.prototype`; show "Browser not supported" message |
| Chrome Android | `AudioContext` auto-suspends on tab hide | Handled by `visibilitychange` handler above |
| Firefox | No `navigator.locks` (Web Locks API) prior to v96 | Check `"locks" in navigator`; skip multi-tab lock silently |
| Safari macOS < 14.1 | No `navigator.locks` | Same fallback as Firefox |
| All mobile | Page may be unloaded entirely when backgrounded | Web Worker fetch loop cannot survive full page unload; on restore, re-initialise from `latest - 1` |
