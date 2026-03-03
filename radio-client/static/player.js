class RadioPlayer extends HTMLElement {
    constructor() {
        super();
        this.attachShadow({ mode: 'open' });

        this.audioCtx = null;
        this.workletNode = null;
        this.worker = null;
        this._releaseLock = null;

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
                .overlay {
                    display: none;
                    position: fixed;
                    top: 0; left: 0; right: 0; bottom: 0;
                    background: rgba(0,0,0,0.8);
                    color: white;
                    align-items: center;
                    justify-content: center;
                    z-index: 1000;
                    font-size: 24px;
                    cursor: pointer;
                }
            </style>
            <div>
                <button id="playBtn" disabled>Loading...</button>
                <div class="status" id="status">Initializing</div>
                <div class="overlay" id="unlockOverlay">Click anywhere to unlock audio</div>
            </div>
        `;
    }

    async connectedCallback() {
        this.token = this.getAttribute('data-token');
        this.isLive = this.getAttribute('data-live') === 'true';

        this.playBtn = this.shadowRoot.getElementById('playBtn');
        this.statusDiv = this.shadowRoot.getElementById('status');
        this.unlockOverlay = this.shadowRoot.getElementById('unlockOverlay');

        if (!this.isLive) {
            this.playBtn.disabled = true;
            this.statusDiv.textContent = 'Stream offline (Manifest unavailable)';
            return;
        }

        // Init Worker
        this.worker = new Worker(new URL('./fetch_worker.js', import.meta.url), { type: 'module' });

        this.worker.onmessage = (e) => {
            if (e.data.type === 'INIT_DONE') {
                console.log("Worker initialized");
            } else if (e.data.type === 'TOKEN_UPDATED') {
                this.token = e.data.token;
            }
        };

        this.worker.postMessage({ type: 'INIT', token: this.token });

        // Multi-Tab Prevention
        navigator.locks.request("radio-player-singleton", async (lock) => {
            this.playBtn.disabled = false;
            this.playBtn.textContent = 'Play Lossless Radio';
            this.statusDiv.textContent = 'Ready';

            // Hold lock indefinitely
            await new Promise(r => this._releaseLock = r);
        }).catch(err => {
            // We ignore errors from lock rejection if any, but `locks.request` usually holds until acquired.
            // If another tab has it, we just don't enable it automatically.
            console.warn("Lock request error:", err);
        });

        // Wait to see if we get the lock or if it's held by someone else
        navigator.locks.query().then(state => {
            const held = state.held.find(h => h.name === 'radio-player-singleton');
            if (held) {
                 // Simple approximation: If we haven't enabled it yet, assume it's transferred.
                 if (this.playBtn.disabled) {
                    this.playBtn.textContent = 'Playback transferred — Click Play';
                    this.statusDiv.textContent = 'Another tab is playing audio.';
                 }
            }
        });


        this.playBtn.addEventListener('click', () => this.togglePlay());
        this.unlockOverlay.addEventListener('click', () => this.unlockAudio());
    }

    async togglePlay() {
        if (!this.audioCtx) {
            // Must create and resume synchronously in click handler
            this.audioCtx = new AudioContext({ sampleRate: 48000 });
            this.audioCtx.resume();

            // iOS Interrupted State Handling
            this.audioCtx.onstatechange = () => {
                if ((this.audioCtx.state === 'suspended' || this.audioCtx.state === 'interrupted') && document.visibilityState === 'visible') {
                    this.audioCtx.resume();
                }
                this.checkAudioState();
            };

            this.checkAudioState();

            try {
                await this.audioCtx.audioWorklet.addModule('/static/worklet.js');
                // The processor name 'radio-processor' must match what's registered in worklet.js
                this.workletNode = new AudioWorkletNode(this.audioCtx, 'radio-processor', {
                    outputChannelCount: [2]
                });
                this.workletNode.connect(this.audioCtx.destination);

                this.workletNode.port.onmessage = (e) => {
                    if (e.data.type === 'OVERFLOW') {
                        console.warn("AudioWorklet Warning:", e.data.message);
                    }
                };

                // Provide MessageChannel to worker for zero-copy transfers
                // The worker will send Float32Array to port1, which will fire port2.onmessage here.
                // We then forward that to the worklet node port.
                const channel = new MessageChannel();

                channel.port2.onmessage = (e) => {
                    // Forward decoded PCM data to the AudioWorklet
                    this.workletNode.port.postMessage(e.data, [e.data.buffer]);
                };

                this.worker.postMessage({ type: 'PLAY', port: channel.port1 }, [channel.port1]);

            } catch (err) {
                this.statusDiv.textContent = 'Failed to load AudioWorklet: ' + err.message;
                return;
            }
        } else {
            if (this.audioCtx.state === 'running') {
                this.audioCtx.suspend();
                this.worker.postMessage({ type: 'STOP' });
                this.playBtn.textContent = 'Play Lossless Radio';
                this.statusDiv.textContent = 'Paused';
                return;
            } else {
                this.audioCtx.resume();
                this.worker.postMessage({ type: 'PLAY' }); // Start fetching again
            }
        }

        this.playBtn.textContent = 'Stop';
        this.statusDiv.textContent = 'Playing...';
    }

    checkAudioState() {
        if (this.audioCtx && this.audioCtx.state === 'suspended') {
            this.unlockOverlay.style.display = 'flex';
        } else {
            this.unlockOverlay.style.display = 'none';
        }
    }

    unlockAudio() {
        if (this.audioCtx) {
            this.audioCtx.resume().then(() => {
                this.unlockOverlay.style.display = 'none';
            });
        }
    }

    disconnectedCallback() {
        if (this._releaseLock) this._releaseLock();
        if (this.worker) this.worker.terminate();
        if (this.audioCtx) this.audioCtx.close();
    }
}

customElements.define('radio-player', RadioPlayer);
