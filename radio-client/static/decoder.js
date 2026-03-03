export class LosslessDecoder {
    constructor() {
        this.decoder = null;
        this.wasmMemory = null;
    }

    async init() {
        // init is an async function exported from flac.js
        const initFlac = (await import('./flac.js')).default;
        const FlacDecoder = (await import('./flac.js')).FlacDecoder;
        const wasm = await initFlac();
        this.wasmMemory = wasm.memory;
        this.decoder = new FlacDecoder();
    }

    reset() {
        if (this.decoder) {
            this.decoder.reset();
        }
    }

    decode(bytes) {
        if (!this.decoder) throw new Error("Decoder not initialized");

        // Push bytes and get a pointer to the decoded Float32Array
        const ptr = this.decoder.push(bytes);
        const len = this.decoder.len();

        if (len === 0) {
            return new Float32Array(0);
        }

        // Create a view into WASM memory
        const wasmView = new Float32Array(this.wasmMemory.buffer, ptr, len);

        // Create a copy so we don't detach WASM memory when posting/transferring
        const pcmCopy = new Float32Array(len);
        pcmCopy.set(wasmView);

        return pcmCopy;
    }
}
