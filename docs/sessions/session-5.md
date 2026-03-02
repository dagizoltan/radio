# Prompt for Session 5: The AudioWorklet Web Component (The Player)

**Goal:** Implement the highly resilient client-side `<radio-player>` Web Component, the dynamic Web Worker fetch loop, and the zero-allocation AudioWorklet processor for flawless browser audio playback.

Ensure you write standard ES modules (pure JavaScript, `.js`).

## 1. The AudioWorklet Processor (`static/worklet.js`)

The Worklet runs on a dedicated, high-priority audio thread. It must never trigger garbage collection.

### 1.1 State and Ring Buffer
```javascript
class RadioProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        // 48000 Hz * 2 channels * 20 seconds = 1,920,000 floats (~7.3 MB)
        this.ringBuffer = new Float32Array(1920000);
        this.writePointer = 0;
        this.readPointer = 0;
        this.samplesAvailable = 0;
        this.isBuffering = true;

        this.port.onmessage = this.handleMessage.bind(this);
    }
    // ...
}
```

### 1.2 Message Handling & Clock Drift Overflow
When the main thread transfers a chunk:
```javascript
handleMessage(event) {
    if (event.data === "FLUSH") {
        this.samplesAvailable = 0;
        this.readPointer = this.writePointer;
        return;
    }
    if (event.data.type === "QUERY_DEPTH") {
        this.port.postMessage({ type: "DEPTH_RESPONSE", samplesAvailable: this.samplesAvailable });
        return;
    }

    const chunk = event.data; // Float32Array

    // Hardware Clock Drift Overflow Protection
    const freeSpace = this.ringBuffer.length - this.samplesAvailable;
    if (chunk.length > freeSpace) {
        // The DAC is playing slower than the ADC is capturing.
        // Drop the chunk entirely to resynchronize clocks.
        this.port.postMessage({ type: "OVERFLOW", dropped: chunk.length });
        // Optional: Post the empty chunk back to the pool
        this.port.postMessage({ type: "RECYCLE", buffer: chunk.buffer }, [chunk.buffer]);
        return;
    }

    // Copy into ring buffer wrapping around
    for (let i = 0; i < chunk.length; i++) {
        this.ringBuffer[this.writePointer] = chunk[i];
        this.writePointer = (this.writePointer + 1) % this.ringBuffer.length;
    }
    this.samplesAvailable += chunk.length;

    // Return the empty buffer to the main thread's pool for zero-allocation
    this.port.postMessage({ type: "RECYCLE", buffer: chunk.buffer }, [chunk.buffer]);
}
```

### 1.3 Zero-Allocation Demux Loop
In `process(inputs, outputs, parameters)`:
```javascript
    // ... state check: if isBuffering or samplesAvailable < 256, return true (output silence).

    const vol = parameters.volume[0] || 0.8;
    const outputL = outputs[0][0];
    const outputR = outputs[0][1];

    for (let i = 0; i < 128; i++) {
        outputL[i] = this.ringBuffer[(this.readPointer + i * 2) % 1920000] * vol;
        outputR[i] = this.ringBuffer[(this.readPointer + i * 2 + 1) % 1920000] * vol;
    }

    this.readPointer = (this.readPointer + 256) % 1920000;
    this.samplesAvailable -= 256;
    return true;
```

## 2. Web Worker Fetch Loop (`static/fetch_worker.js`)

To prevent the browser from throttling `setInterval` when the tab is backgrounded, the fetch loop must live in a Web Worker.

### 2.1 Dynamic Buffering Strategy
1. The Worker receives a `START` message with `r2Url`, `quality`, and `token`.
2. It polls `/api/manifest` every `segment_s / 2` seconds.
3. **Bandwidth Tracking:**
   - Record `startTime = performance.now()`.
   - `fetch(segmentUrl)`.
   - Record `endTime = performance.now()`.
   - Calculate `bps = (bytesDownloaded * 8) / ((endTime - startTime) / 1000)`.
   - Maintain a moving average of the last 3 chunks.
   - If `avg_bps < 1500000` (for HQ) or `avg_bps < 800000` (for LQ), increase the target buffer depth (pre-roll) from `2` segments to `4` segments.
4. **Jump-Ahead Logic:**
   - Default: `currentIndex = Math.max(0, latest - 2)`.
   - If `currentIndex < latest - current_target_depth - 1`, snap to `latest - 1`.
5. **Zero-Copy Pool Transfer:**
   - The worker imports the WASM module.
   - It reads the `ReadableStream` chunks, pushes them to `wasm.push()`.
   - It extracts the memory view: `new Float32Array(wasm.memory.buffer, wasm.get_ptr(), wasm.get_len())`.
   - **Crucial:** It maintains an array of recycled buffers `const pool = []` populated by `RECYCLE` messages forwarded from the worklet.
   - `const pcmCopy = pool.pop() || new Float32Array(pcmView.length);`
   - `pcmCopy.set(pcmView);`
   - `postMessage({ type: "PCM", chunk: pcmCopy }, [pcmCopy.buffer]);`

## 3. Player Web Component (`static/player.js`)

### 3.1 Multi-Tab Prevention (Web Locks API)
In the `connectedCallback`:
```javascript
navigator.locks.request("radio-player-singleton", async (lock) => {
    // We have the lock! Hide the "in use" warning and enable the Play button.
    this._enablePlayButton();
    // Hold the lock indefinitely until the tab closes
    await new Promise(resolve => this._releaseLock = resolve);
}).catch(console.error);

// If the callback fires asynchronously later (meaning another tab closed and yielded the lock),
// DO NOT call audioCtx.resume() or auto-play. That violates Autoplay Policy.
// Show UI: "Playback transferred — Click Play".
```

### 3.2 AudioContext Autoplay Policy
In the Play button click handler:
```javascript
// MUST be synchronous within the click handler
const audioCtx = new AudioContext({ sampleRate: 48000 });
audioCtx.resume();

try {
    await audioCtx.audioWorklet.addModule('/static/worklet.js');
} catch(e) {
    this._showError("Audio engine failed to load");
    return;
}
```

### 3.3 Visibility Change & iOS Interrupted State
```javascript
document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
        if (audioCtx.state === "suspended" || audioCtx.state === "interrupted") {
            audioCtx.resume();
        }
        // Send QUERY_DEPTH to worklet. If response > 48000 * 15 (15 seconds stale),
        // send FLUSH to worklet and JUMP_LATEST to fetch_worker.
    }
});
```

## Validation
Load the UI. Click Play. Confirm the lock is held (open a second tab, verify it shows "Stream in use"). In the first tab, toggle from HQ to LQ; confirm the fetch worker correctly changes the URL and resets the decoder without a page reload or audible pop. Background the tab on a mobile device for 60 seconds, then return; confirm the player flushes the stale 60 seconds and instantly jumps to the live edge.
