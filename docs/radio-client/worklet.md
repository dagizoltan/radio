# AudioWorklet (islands/worklet.ts)

The `islands/worklet.ts` file defines the `AudioWorkletProcessor` that runs on the browser's dedicated audio rendering thread.

## Processor Registration

The file registers the processor using `registerProcessor("radio-processor", class extends AudioWorkletProcessor { ... })`.

## State and Parameters

The class maintains internal state:
*   A queue data structure (e.g., an array of `Float32Array` chunks).
*   A read offset integer tracking the current position within the frontmost chunk.

It exposes a `"volume"` parameter via the `parameterDescriptors` static getter, with a default value of `0.8` (range `0.0` to `1.0`).

## Port Messages

The main thread sends `Float32Array` chunks (interleaved stereo PCM from the WASM decoder) to the worklet via `this.port.postMessage`.

The worklet implements `this.port.onmessage = (event) => { ... }` to receive these chunks and push them onto the back of its internal queue.

## Output Processing

The `process(inputs, outputs, parameters)` method is called by the Web Audio API every time it needs more audio data.

1.  **Output Block:** The Web Audio API always requests blocks of exactly 128 frames per channel.
2.  **Underrun Protection:** The worklet checks if it has enough data. Since the stream is interleaved stereo, 128 frames = 256 samples.
    *   If the total number of samples across all chunks in the queue (minus the read offset) is less than 256, it signifies a buffer underrun. The method outputs silence (zeroes) and returns `true` to keep the processor alive.
3.  **Demux and Volume:** If enough data exists, it pulls exactly 256 samples from the queue.
    *   It iterates 128 times.
    *   Even indices map to the left output channel: `outputs[0][0][i]`.
    *   Odd indices map to the right output channel: `outputs[0][1][i]`.
    *   Each sample is multiplied by the current `parameters.volume[0]` value.
4.  **Queue Management:** As samples are read, the internal read offset advances. If the offset reaches the end of the frontmost chunk, that chunk is shifted off the queue (`queue.shift()`), and the offset is reset to 0.
5.  **Keep Alive:** Returns `true` to ensure the worklet continues to be called.

## Critical Constraints

**CRITICAL CONSTRAINT:** The browser audio player must use `AudioWorkletNode` for audio output, not `ScriptProcessorNode` (deprecated) or direct buffer scheduling. The worklet runs on a dedicated thread separate from the main JS thread.