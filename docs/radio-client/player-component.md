# Player Web Component (islands/player.ts)

The `islands/player.ts` file defines the interactive client-side logic for the `<radio-player>` Web Component.

## Initialization

The component is defined using `customElements.define("radio-player", class extends HTMLElement { ... })`. It does not use a frontend framework.

### connectedCallback

When the element is inserted into the DOM, `connectedCallback` fires:

1.  **Render:** It populates `this.innerHTML` with the player's UI (if not already hydrated by SSR):
    *   A `<canvas>` element for the waveform.
    *   A metadata section displaying the title and a Quality Selector toggle (e.g., `<select>` or buttons for `HQ` / `LQ`).
    *   A controls row containing a play/stop button, an animated live indicator dot, a latency display, and a volume `<input type="range">`.
2.  **Event Binding:** It attaches click listeners to the play/stop button, input listeners to the volume slider, and change listeners to the Quality Selector.

## Playback Sequence

When the user clicks the "Play" button, the following sequence occurs:

1.  **AudioContext:** Create a new `AudioContext` with `{ sampleRate: 44100 }`.
2.  **Worklet Loading:** Await `audioCtx.audioWorklet.addModule("/static/worklet.js")`.
3.  **Node Creation:** Create an `AudioWorkletNode` named `"radio-processor"`. Pass the initial volume from the slider as a parameter.
4.  **Analyser Chain:** Create an `AnalyserNode` for the waveform visualizer. Chain them: `workletNode.connect(analyserNode).connect(audioCtx.destination)`.
5.  **WASM Import:** Dynamically import the WASM decoder module:
    ```javascript
    import init, { FlacDecoder } from "/static/flac_decoder.js";
    await init();
    const decoder = new FlacDecoder();
    ```
6.  **Fetch Loop:** Start the main fetch loop and the waveform animation loop (`requestAnimationFrame`).

## Fetch Loop Algorithm

The core of the player is the fetch loop, which continuously polls for new segments and streams them to the decoder.

1.  **Manifest Poll:** Fetch `/manifest.json`.
    *   If offline, update the UI and retry after a delay.
    *   If live, extract `latest` segment index and `segment_s` duration.
2.  **Buffering Strategy:** Start playing 2 segments behind the `latest` index to build a small buffer against network jitter.
3.  **Jump-Ahead Logic:**
    *   If the player's current segment index is ahead of `latest`, sleep for `segment_s / 2` and repoll.
    *   If the player falls more than 3 segments behind `latest` (e.g., due to pausing or network stall), immediately jump to `latest - 1`.
4.  **Segment Streaming:**
    *   Fetch `/segment/${currentQuality}/${currentIndex}` (e.g., `hq` or `lq` based on UI state).
    *   Get a `ReadableStreamDefaultReader` from the response body.
    *   Loop `reader.read()`. As each `Uint8Array` chunk arrives:
        *   Pass the chunk to the WASM decoder: `const pcm = decoder.push(chunk)`.
        *   If `pcm` (an `Float32Array`) has length > 0, post it to the worklet: `workletNode.port.postMessage(pcm, [pcm.buffer])` (transferring ownership for performance).
5.  **Quality Switching:** If the user changes the quality mid-stream:
    *   The current `reader.cancel()` is called.
    *   A `"FLUSH"` message is sent to the `AudioWorklet` via `postMessage` to instantly clear any buffered PCM data. This prevents an audible pitch-shift or pop when the new codec chunks arrive.
    *   The `currentQuality` state updates.
    *   The fetch loop immediately attempts to fetch the *same* `currentIndex` using the new quality path (`hq` FLAC or `lq` MP3).
6.  **Iteration:** When `reader.read()` returns `done: true` normally, increment the `currentIndex`.
7.  **Latency Display:** Calculate and update the UI with the estimated latency: `(latest - currentIndex) * segment_s` seconds behind live.