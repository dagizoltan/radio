class RadioProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        // Ring buffer sized for ~20 seconds of stereo audio at 48kHz
        // 48000 samples/sec * 20 sec * 2 channels = 1,920,000 floats
        this.RING_BUFFER_SIZE = 1920000;
        this.ringBuffer = new Float32Array(this.RING_BUFFER_SIZE);

        this.readPointer = 0;
        this.writePointer = 0;
        this.samplesAvailable = 0;

        this.volume = 1.0;

        this.port.onmessage = (event) => {
            const data = event.data;

            if (data === "FLUSH") {
                this.readPointer = 0;
                this.writePointer = 0;
                this.samplesAvailable = 0;
                return;
            }

            if (data === "QUERY_DEPTH") {
                this.port.postMessage({ type: "DEPTH", samplesAvailable: this.samplesAvailable });
                return;
            }

            if (data instanceof Float32Array) {
                const chunk = data;
                const freeSpace = this.RING_BUFFER_SIZE - this.samplesAvailable;

                // Clock Drift Protection
                if (chunk.length > freeSpace) {
                    this.port.postMessage({ type: "OVERFLOW", message: "Ring buffer overflow" });
                    return; // Drop chunk
                }

                // Write chunk to ring buffer
                for (let i = 0; i < chunk.length; i++) {
                    this.ringBuffer[this.writePointer] = chunk[i];
                    this.writePointer = (this.writePointer + 1) % this.RING_BUFFER_SIZE;
                }
                this.samplesAvailable += chunk.length;
            }
        };
    }

    process(inputs, outputs, parameters) {
        const output = outputs[0];
        const channelLeft = output[0];
        const channelRight = output[1];

        // Output silence if underrun (need 128 stereo frames = 256 samples)
        if (this.samplesAvailable < 256) {
            return true;
        }

        // Zero-Allocation Demux
        for (let i = 0; i < 128; i++) {
            channelLeft[i] = this.ringBuffer[(this.readPointer + i * 2) % this.RING_BUFFER_SIZE] * this.volume;
            channelRight[i] = this.ringBuffer[(this.readPointer + i * 2 + 1) % this.RING_BUFFER_SIZE] * this.volume;
        }

        this.readPointer = (this.readPointer + 256) % this.RING_BUFFER_SIZE;
        this.samplesAvailable -= 256;

        return true;
    }
}

registerProcessor('radio-processor', RadioProcessor);
