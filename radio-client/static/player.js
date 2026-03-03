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
                    border: 1px solid #ccc;
                    border-radius: 4px;
                    background: #f8f9fa;
                    transition: background 0.2s, color 0.2s;
                }
                button:disabled {
                    opacity: 0.6;
                    cursor: not-allowed;
                    background: #e9ecef;
                }
                button:hover:not(:disabled) {
                    background: #e2e6ea;
                }
                .controls-container {
                    display: flex;
                    align-items: center;
                    gap: 15px;
                    margin-top: 15px;
                    flex-wrap: wrap;
                }
                .status {
                    margin-top: 15px;
                    color: #495057;
                    font-size: 14px;
                }
                .buffering {
                    animation: pulse 1.5s infinite;
                }
                @keyframes pulse {
                    0% { opacity: 0.6; }
                    50% { opacity: 1; }
                    100% { opacity: 0.6; }
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

                <div class="controls-container">
                    <button id="muteBtn" disabled>Mute</button>
                    <input type="range" id="volumeSlider" min="0" max="1" step="0.01" value="1" disabled>
                </div>
            </div>
        `;
    }

    async connectedCallback() {
        this.token = this.getAttribute('data-token');
        this.isLive = this.getAttribute('data-live') === 'true';

        this.playBtn = this.shadowRoot.getElementById('playBtn');
        this.statusDiv = this.shadowRoot.getElementById('status');
        this.unlockOverlay = this.shadowRoot.getElementById('unlockOverlay');
        this.muteBtn = this.shadowRoot.getElementById('muteBtn');
        this.volumeSlider = this.shadowRoot.getElementById('volumeSlider');

        this.setupSSE();

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
            this.muteBtn.disabled = false;
            this.volumeSlider.disabled = false;
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

        // Volume controls handlers
        this.volumeSlider.addEventListener('input', (e) => this.setVolume(e.target.value));
        this.muteBtn.addEventListener('click', () => this.toggleMute());

        this.setupMediaSession();
    }

    setVolume(value) {
        this.currentVolume = parseFloat(value);

        // If adjusting volume while muted, automatically unmute.
        if (this.muteBtn.textContent === 'Unmute') {
            this.muteBtn.textContent = 'Mute';
        }

        if (this.workletNode) {
            this.workletNode.port.postMessage({ type: 'SET_VOLUME', volume: this.currentVolume });
        }
    }

    toggleMute() {
        if (this.muteBtn.textContent === 'Mute') {
            this.muteBtn.textContent = 'Unmute';
            if (this.workletNode) {
                this.workletNode.port.postMessage({ type: 'SET_VOLUME', volume: 0 });
            }
        } else {
            this.muteBtn.textContent = 'Mute';
            if (this.workletNode) {
                this.workletNode.port.postMessage({ type: 'SET_VOLUME', volume: this.volumeSlider.value });
            }
        }
    }

    setupMediaSession() {
        if ('mediaSession' in navigator) {
            navigator.mediaSession.metadata = new MediaMetadata({
                title: 'Lossless Web Radio',
                artist: 'Live Stream',
                album: 'Radio Server'
            });

            navigator.mediaSession.setActionHandler('play', () => {
                if (!this.audioCtx || this.audioCtx.state === 'suspended') {
                    this.togglePlay();
                }
            });

            navigator.mediaSession.setActionHandler('pause', () => {
                if (this.audioCtx && this.audioCtx.state === 'running') {
                    this.togglePlay();
                }
            });

            navigator.mediaSession.setActionHandler('stop', () => {
                if (this.audioCtx && this.audioCtx.state === 'running') {
                    this.togglePlay();
                }
            });
        }
    }

    updateMediaSessionState(state) {
        if ('mediaSession' in navigator) {
            navigator.mediaSession.playbackState = state;
        }
    }

    setupSSE() {
        this.eventSource = new EventSource('/api/events');

        this.eventSource.onmessage = (event) => {
            try {
                // Try parsing as JSON first, since we might pass JSON from the server
                // Alternatively, backend might pass text strings. For robustness:
                let msg = event.data;

                // For this session, we assume the server sends now-playing text directly
                // or we can parse if it's JSON.
                let title = msg;
                if (msg.startsWith('{')) {
                    const data = JSON.parse(msg);
                    if (data.title) title = data.title;
                }

                if (title && title !== 'keepalive') {
                    this.statusDiv.textContent = `Playing: ${title}`;
                    this.statusDiv.classList.remove('buffering');

                    if ('mediaSession' in navigator) {
                        navigator.mediaSession.metadata = new MediaMetadata({
                            title: title,
                            artist: 'Live Stream',
                            album: 'Radio Server'
                        });
                    }
                }
            } catch (err) {
                console.error("Error processing SSE message:", err);
            }
        };

        this.eventSource.onerror = (err) => {
            console.warn("SSE Error, reconnecting...", err);
            // EventSource auto-reconnects, so we don't strictly need to do anything
        };
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
                this.statusDiv.classList.remove('buffering');
                this.updateMediaSessionState('paused');
                return;
            } else {
                this.audioCtx.resume();
                this.worker.postMessage({ type: 'PLAY' }); // Start fetching again
            }
        }

        this.playBtn.textContent = 'Stop';
        this.statusDiv.textContent = 'Playing...';
        this.statusDiv.classList.add('buffering');
        this.updateMediaSessionState('playing');
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
        if (this.eventSource) this.eventSource.close();
        if (this._releaseLock) this._releaseLock();
        if (this.worker) this.worker.terminate();
        if (this.audioCtx) this.audioCtx.close();
    }
}

customElements.define('radio-player', RadioPlayer);
