# AudioWorklet (islands/worklet.ts)

The `islands/worklet.ts` file defines the `AudioWorkletProcessor` that runs on the browser's dedicated audio rendering thread.

## Processor Registration

The file registers the processor using `registerProcessor("radio-processor", class extends AudioWorkletProcessor { ... })`.

## State and Parameters

The class maintains internal state using a pre-allocated **Ring Buffer** to achieve zero-allocations and avoid JavaScript Garbage Collection (GC) pauses:
*   `ringBuffer`: A single, large `Float32Array` pre-allocated to hold several seconds of audio (e.g., 5 seconds = `44100 * 2 channels * 5` elements).
*   `writePointer`: An integer tracking where the next incoming chunk should be written.
*   `readPointer`: An integer tracking where the next playback frame should be read.
*   `samplesAvailable`: An integer tracking how much unread data exists in the ring buffer.

It exposes a `"volume"` parameter via the `parameterDescriptors` static getter, with a default value of `0.8` (range `0.0` to `1.0`).

## Port Messages

The main thread sends `Float32Array` chunks (interleaved stereo PCM from the WASM decoder) to the worklet via `this.port.postMessage`.

The worklet implements `this.port.onmessage = (event) => { ... }` to receive these messages.
*   If the message contains a `Float32Array`, it copies the chunk into the `ringBuffer` starting at the `writePointer`, wrapping around to index 0 if it hits the end of the array. It increments `samplesAvailable`.
*   If the message is a specific string command (e.g., `"FLUSH"`), it immediately zeroes out the `samplesAvailable` and aligns the read/write pointers. This is critical for preventing audio artifacts when the user switches stream qualities mid-playback.

## Output Processing

The `process(inputs, outputs, parameters)` method is called by the Web Audio API every time it needs more audio data.

1.  **Output Block:** The Web Audio API always requests blocks of exactly 128 frames per channel.
2.  **Underrun Protection:** The worklet checks if it has enough data. Since the stream is interleaved stereo, 128 frames = 256 samples.
    *   If `samplesAvailable < 256`, it signifies a buffer underrun. The method outputs silence (zeroes) and returns `true` to keep the processor alive.
3.  **Demux and Volume:** If enough data exists, it pulls exactly 256 samples from the `ringBuffer` starting at `readPointer`, wrapping around if necessary.
    *   It iterates 128 times.
    *   Even indices map to the left output channel: `outputs[0][0][i]`.
    *   Odd indices map to the right output channel: `outputs[0][1][i]`.
    *   Each sample is multiplied by the current `parameters.volume[0]` value.
4.  **Queue Management:** As samples are read, the `readPointer` advances and `samplesAvailable` decreases. This zero-allocation method guarantees the GC is never provoked on the audio thread.
5.  **Keep Alive:** Returns `true` to ensure the worklet continues to be called.

## Critical Constraints

**CRITICAL CONSTRAINT:** The browser audio player must use `AudioWorkletNode` for audio output, not `ScriptProcessorNode` (deprecated) or direct buffer scheduling. The worklet runs on a dedicated thread separate from the main JS thread.