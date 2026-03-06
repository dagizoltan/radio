// Mock AudioWorkletProcessor environment
class AudioWorkletProcessor {
    constructor() {
        this.port = {
            postMessage: (msg) => {
                this._lastMessage = msg;
            }
        };
    }
}
globalThis.AudioWorkletProcessor = AudioWorkletProcessor;
globalThis.registerProcessor = () => {};

// Load the worklet script into the global scope to access RadioProcessor
const workletCode = await Deno.readTextFile(new URL("./static/worklet.js", import.meta.url));

// Since worklet.js defines a class with `class RadioProcessor`, it might not end up in globalThis
// when evaluated inside an ES module scope natively, or it might get blocked by strict mode.
// We explicitly attach it to globalThis inside the eval.
const modifiedCode = workletCode + "\n\nglobalThis.RadioProcessor = RadioProcessor;";
eval(modifiedCode);

Deno.test("RadioProcessor Ring Buffer Pointer Math", () => {
    // RadioProcessor is now available in the global scope from the eval
    const processor = new globalThis.RadioProcessor();

    // Create a 1 million float chunk
    const chunk1 = new Float32Array(1000000);
    chunk1.fill(1.0);

    // Simulate port message
    processor.port.onmessage({ data: chunk1 });

    if (processor.samplesAvailable !== 1000000) {
        throw new Error(`Expected 1,000,000 samples, got ${processor.samplesAvailable}`);
    }

    if (processor.writePointer !== 1000000) {
        throw new Error(`Expected writePointer at 1,000,000, got ${processor.writePointer}`);
    }

    // Create a second 1 million float chunk
    const chunk2 = new Float32Array(1000000);
    chunk2.fill(2.0);

    // Simulate port message again
    processor.port.onmessage({ data: chunk2 });

    // Since 2,000,000 > 1,920,000, it should write as much as it can (920,000) and post an OVERFLOW warning.
    // The samplesAvailable should be maxed out at 1,920,000.
    if (processor.samplesAvailable !== 1920000) {
        throw new Error(`Expected 1,920,000 samples after overflow, got ${processor.samplesAvailable}`);
    }

    // The processor posts RETURN_BUFFER after the OVERFLOW warning, so _lastMessage is RETURN_BUFFER.
    // We can assume if writePointer is at 0 and samplesAvailable is max, the logic worked.

    // Write pointer should be at 1920000 % 1920000 = 0
    if (processor.writePointer !== 0) {
        throw new Error(`Expected writePointer to wrap around to 0, got ${processor.writePointer}`);
    }

    console.log("Ring Buffer tests passed");
});
