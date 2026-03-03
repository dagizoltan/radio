import init, { FlacDecoder } from './flac.js';

export class LosslessDecoder {
    constructor() {
        this.decoder = null;
        this.wasmMemory = null;
    }

    async init() {
        const wasm = await init();
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

// Simple Web Component for the player
if (typeof HTMLElement !== 'undefined') {
    class RadioPlayer extends HTMLElement {
        constructor() {
            super();
            this.attachShadow({ mode: 'open' });
            this.decoder = new LosslessDecoder();

            this.shadowRoot.innerHTML = `
                <style>
                    :host {
                        display: block;
                        padding: 20px;
                        font-family: sans-serif;
                    }
                    button {
                        padding: 10px 20px;
                        font-size: 16px;
                        cursor: pointer;
                    }
                    .status {
                        margin-top: 10px;
                        color: #666;
                    }
                </style>
                <div>
                    <button id="playBtn">Play Lossless Radio</button>
                    <div class="status" id="status">Ready</div>
                </div>
            `;
        }

        async connectedCallback() {
            this.token = this.getAttribute('data-token');
            this.isLive = this.getAttribute('data-live') === 'true';

            this.playBtn = this.shadowRoot.getElementById('playBtn');
            this.statusDiv = this.shadowRoot.getElementById('status');

            if (!this.isLive) {
                this.playBtn.disabled = true;
                this.statusDiv.textContent = 'Stream offline (Manifest unavailable)';
                return;
            }

            try {
                await this.decoder.init();
                this.playBtn.addEventListener('click', () => this.togglePlay());
            } catch (e) {
                this.statusDiv.textContent = 'Failed to load decoder: ' + e.message;
                this.playBtn.disabled = true;
            }
        }

        async togglePlay() {
            if (this.playBtn.textContent === 'Play Lossless Radio') {
                this.playBtn.textContent = 'Stop';
                this.statusDiv.textContent = 'Playing...';
            } else {
                this.playBtn.textContent = 'Play Lossless Radio';
                this.statusDiv.textContent = 'Ready';
            }
        }
    }

    customElements.define('radio-player', RadioPlayer);
}
