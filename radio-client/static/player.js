class RadioPlayer extends HTMLElement {
    constructor() {
        super();
        this.attachShadow({ mode: 'open' });

        this.audioCtx = null;
        this.workletNode = null;
        this.analyser = null;
        this.worker = null;
        this._releaseLock = null;
        this.currentVolume = 1.0;
        this.isVisualizerRunning = false;

        this.shadowRoot.innerHTML = `
            <style>
                @import url('https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;500;600;700&display=swap');

                :host {
                    display: block;
                    font-family: 'Outfit', sans-serif;
                    color: #f8fafc;
                }

                .player-card {
                    max-width: 440px;
                    margin: 40px auto;
                    background: rgba(15, 23, 42, 0.7);
                    backdrop-filter: blur(20px);
                    border: 1px solid rgba(255, 255, 255, 0.1);
                    border-radius: 32px;
                    padding: 32px;
                    box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5);
                    position: relative;
                    overflow: hidden;
                }

                .player-card::before {
                    content: '';
                    position: absolute;
                    top: -50%;
                    left: -50%;
                    width: 200%;
                    height: 200%;
                    background: radial-gradient(circle at center, rgba(99, 102, 241, 0.08) 0%, transparent 50%);
                    pointer-events: none;
                }

                .header {
                    display: flex;
                    justify-content: space-between;
                    align-items: center;
                    margin-bottom: 24px;
                }

                .brand {
                    font-weight: 700;
                    font-size: 0.85rem;
                    letter-spacing: 0.1em;
                    text-transform: uppercase;
                    color: #6366f1;
                }

                .live-badge {
                    background: rgba(34, 197, 94, 0.1);
                    color: #22c55e;
                    border: 1px solid rgba(34, 197, 94, 0.2);
                    padding: 4px 10px;
                    border-radius: 50px;
                    font-size: 0.7rem;
                    font-weight: 700;
                    display: flex;
                    align-items: center;
                    gap: 6px;
                }

                .live-badge .dot {
                    width: 6px;
                    height: 6px;
                    background: #22c55e;
                    border-radius: 50%;
                    box-shadow: 0 0 10px #22c55e;
                }

                .offline-badge {
                    background: rgba(255, 255, 255, 0.05);
                    color: #94a3b8;
                    padding: 4px 10px;
                    border-radius: 50px;
                    font-size: 0.7rem;
                    font-weight: 700;
                }

                .visualizer-container {
                    height: 80px;
                    margin-bottom: 24px;
                    display: flex;
                    align-items: flex-end;
                    justify-content: center;
                    gap: 3px;
                }

                canvas {
                    width: 100%;
                    height: 100%;
                }

                .info {
                    text-align: center;
                    margin-bottom: 32px;
                }

                .title {
                    font-size: 1.5rem;
                    font-weight: 700;
                    margin-bottom: 4px;
                    background: linear-gradient(to right, #fff, #94a3b8);
                    -webkit-background-clip: text;
                    -webkit-text-fill-color: transparent;
                }

                .subtitle {
                    font-size: 0.9rem;
                    color: #94a3b8;
                    font-weight: 500;
                }

                .controls {
                    display: flex;
                    flex-direction: column;
                    gap: 24px;
                }

                .main-btns {
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    gap: 24px;
                }

                .quality-select {
                    background: rgba(255,255,255,0.05);
                    border: 1px solid rgba(255,255,255,0.1);
                    color: #94a3b8;
                    font-family: inherit;
                    font-size: 0.75rem;
                    padding: 4px 8px;
                    border-radius: 8px;
                    cursor: pointer;
                    outline: none;
                }

                .play-btn {
                    width: 64px;
                    height: 64px;
                    border-radius: 50%;
                    background: #6366f1;
                    color: white;
                    border: none;
                    cursor: pointer;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    box-shadow: 0 10px 20px rgba(99, 102, 241, 0.4);
                    transition: all 0.2s;
                }

                .play-btn:hover:not(:disabled) {
                    transform: scale(1.05);
                    background: #4f46e5;
                }

                .play-btn:disabled {
                    background: #334155;
                    box-shadow: none;
                    cursor: not-allowed;
                }

                .play-btn svg { width: 28px; height: 28px; fill: currentColor; }

                .volume-row {
                    display: flex;
                    align-items: center;
                    gap: 16px;
                }

                .volume-slider {
                    flex: 1;
                    -webkit-appearance: none;
                    background: rgba(255, 255, 255, 0.1);
                    height: 4px;
                    border-radius: 2px;
                    outline: none;
                }

                .volume-slider::-webkit-slider-thumb {
                    -webkit-appearance: none;
                    width: 14px;
                    height: 14px;
                    background: #6366f1;
                    border-radius: 50%;
                    cursor: pointer;
                    box-shadow: 0 0 10px rgba(99, 102, 241, 0.5);
                }

                .overlay {
                    display: none;
                    position: absolute;
                    top: 0; left: 0; right: 0; bottom: 0;
                    background: rgba(15, 23, 42, 0.9);
                    z-index: 100;
                    align-items: center;
                    justify-content: center;
                    text-align: center;
                    padding: 20px;
                    cursor: pointer;
                    border-radius: 32px;
                }

                .status-text {
                    font-size: 0.8rem;
                    font-weight: 600;
                    color: #6366f1;
                    text-align: center;
                    margin-top: 12px;
                    letter-spacing: 0.05em;
                }
            </style>
            
            <div class="player-card">
                <div class="header">
                    <div class="brand">Antigravity Radio</div>
                    <div id="statusBadge">
                        <div class="offline-badge">LOADING</div>
                    </div>
                </div>

                <div class="visualizer-container">
                    <canvas id="visualizer"></canvas>
                </div>

                <div class="info">
                    <div class="title" id="trackTitle">Ready to stream</div>
                    <div class="subtitle" id="trackInfo">Lossless Audio Pipeline</div>
                </div>

                <div class="controls">
                    <div class="main-btns">
                        <button class="play-btn" id="playBtn" disabled>
                            <svg viewBox="0 0 24 24" id="playIcon"><path d="M8 5v14l11-7z"/></svg>
                        </button>
                        <select class="quality-select" id="qualitySelect">
                            <option value="auto">Auto</option>
                            <option value="hq">High (FLAC)</option>
                            <option value="lq">Low (FLAC)</option>
                        </select>
                    </div>

                    <div class="volume-row">
                        <svg viewBox="0 0 24 24" width="20" height="20" fill="#94a3b8"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/></svg>
                        <input type="range" class="volume-slider" id="volumeSlider" min="0" max="1" step="0.01" value="1">
                    </div>

                    <div class="status-text" id="statusText">System Ready</div>
                </div>

                <div class="overlay" id="unlockOverlay">
                    <div>
                        <div style="font-size: 1.5rem; font-weight: 700; margin-bottom: 8px;">Tap to Connect</div>
                        <div style="color: #94a3b8; font-size: 0.9rem;">Audio interaction required</div>
                    </div>
                </div>
            </div>
        `;

        this.canvas = this.shadowRoot.getElementById('visualizer');
        this.canvasCtx = this.canvas.getContext('2d');
    }

    async connectedCallback() {
        this.token = this.getAttribute('data-token');
        this.isLive = this.getAttribute('data-live') === 'true';
        this.r2Url = this.getAttribute('data-r2-url');
        this.eventsUrl = this.getAttribute('data-events-url');

        this.playBtn = this.shadowRoot.getElementById('playBtn');
        this.statusBadge = this.shadowRoot.getElementById('statusBadge');
        this.statusText = this.shadowRoot.getElementById('statusText');
        this.unlockOverlay = this.shadowRoot.getElementById('unlockOverlay');
        this.volumeSlider = this.shadowRoot.getElementById('volumeSlider');
        this.trackTitle = this.shadowRoot.getElementById('trackTitle');
        this.qualitySelect = this.shadowRoot.getElementById('qualitySelect');

        this.setupSSE();
        this.setupMediaSession();

        if (!this.isLive) {
            this.updateBadge('OFFLINE');
        } else {
            this.initWorker();
        }

        this.playBtn.addEventListener('click', () => this.togglePlay());
        this.unlockOverlay.addEventListener('click', () => this.unlockAudio());
        this.volumeSlider.addEventListener('input', (e) => this.setVolume(e.target.value));

        this.resizeCanvas();
        window.addEventListener('resize', () => this.resizeCanvas());
    }

    initWorker() {
        if (this.worker) return;

        this.worker = new Worker(new URL('./fetch_worker.js', import.meta.url), { type: 'module' });
        this.worker.onmessage = (e) => {
            if (e.data.type === 'TOKEN_UPDATED') this.token = e.data.token;
            if (e.data.type === 'STALE_MANIFEST') {
                if (e.data.isStale && this.audioCtx?.state === 'running') {
                    this.statusText.textContent = 'Stream unstable...';
                }
            }
            if (e.data.type === 'RECONNECTING') {
                this.statusText.textContent = 'Reconnecting...';
            }
        };

        this.worker.postMessage({ type: 'INIT', token: this.token, r2Url: this.r2Url });

        this.qualitySelect.addEventListener('change', (e) => {
            if (this.worker) {
                this.worker.postMessage({ type: 'SET_QUALITY', quality: e.target.value });
            }
        });

        navigator.locks.request("radio-player-singleton", async (lock) => {
            this.playBtn.disabled = false;
            this.updateBadge('LIVE');
            await new Promise(r => this._releaseLock = r);
        });
    }

    resizeCanvas() {
        this.canvas.width = this.canvas.offsetWidth * window.devicePixelRatio;
        this.canvas.height = this.canvas.offsetHeight * window.devicePixelRatio;
    }

    updateBadge(status) {
        if (status === 'LIVE') {
            this.statusBadge.innerHTML = '<div class="live-badge"><div class="dot"></div>LIVE</div>';
        } else {
            this.statusBadge.innerHTML = `<div class="offline-badge">${status}</div>`;
        }
    }

    setupSSE() {
        if (!this.eventsUrl) return;
        this.eventSource = new EventSource(this.eventsUrl);
        this.eventSource.onmessage = (event) => {
            let msg = event.data;
            if (msg === 'keepalive') return;
            try {
                if (msg.startsWith('{')) {
                    const data = JSON.parse(msg);
                    if (data.type === 'metrics') {
                        if (!this.isLive) {
                            this.isLive = true;
                            this.updateBadge('LIVE');
                            this.initWorker();
                        }
                    }
                } else {
                    this.trackTitle.textContent = msg;
                    this.statusText.textContent = 'Now Playing';
                    this.updateMediaSessionMetadata(msg);
                }
            } catch (err) { }
        };
    }

    setupMediaSession() {
        if ('mediaSession' in navigator) {
            navigator.mediaSession.playbackState = 'none';
            navigator.mediaSession.setActionHandler('play', () => this.togglePlay());
            navigator.mediaSession.setActionHandler('pause', () => this.togglePlay());
            this.updateMediaSessionMetadata("Ready to stream");
        }
    }

    updateMediaSessionMetadata(title) {
        if ('mediaSession' in navigator) {
            navigator.mediaSession.metadata = new MediaMetadata({
                title: title,
                artist: 'Antigravity Radio',
                album: 'Live Lossless Broadcast'
            });
        }
    }

    async togglePlay() {
        if (!this.audioCtx) {
            this.audioCtx = new AudioContext({ sampleRate: 48000 });
            this.analyser = this.audioCtx.createAnalyser();
            this.analyser.fftSize = 64;
            this.analyser.connect(this.audioCtx.destination);

            this.audioCtx.onstatechange = () => this.checkAudioState();

            try {
                await this.audioCtx.audioWorklet.addModule('/static/worklet.js');
                this.workletNode = new AudioWorkletNode(this.audioCtx, 'radio-processor', {
                    outputChannelCount: [2]
                });
                this.workletNode.connect(this.analyser);

                this.workletNode.port.postMessage({ type: 'SET_VOLUME', volume: this.currentVolume });

                const channel = new MessageChannel();
                channel.port2.onmessage = (e) => {
                    this.workletNode.port.postMessage(e.data, [e.data.buffer]);
                };
                this.worker.postMessage({ type: 'PLAY', port: channel.port1 }, [channel.port1]);

                this.renderVisualizer();
            } catch (err) {
                this.statusText.textContent = 'Hardware Error';
                return;
            }
        }

        if (this.audioCtx.state === 'running') {
            this.audioCtx.suspend();
            this.worker.postMessage({ type: 'STOP' });
            this.updatePlayState(false);
        } else {
            this.audioCtx.resume();
            this.worker.postMessage({ type: 'PLAY' });
            this.updatePlayState(true);
        }
    }

    updatePlayState(isPlaying) {
        const icon = this.shadowRoot.getElementById('playIcon');
        if (isPlaying) {
            icon.innerHTML = '<path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/>';
            this.statusText.textContent = 'Streaming Lossless';
            if ('mediaSession' in navigator) navigator.mediaSession.playbackState = 'playing';
            if (!this.isVisualizerRunning) {
                this.renderVisualizer();
            }
        } else {
            icon.innerHTML = '<path d="M8 5v14l11-7z"/>';
            this.statusText.textContent = 'Paused';
            if ('mediaSession' in navigator) navigator.mediaSession.playbackState = 'paused';
            this.isVisualizerRunning = false;
        }
    }

    renderVisualizer() {
        if (!this.analyser) return;
        this.isVisualizerRunning = true;
        const bufferLength = this.analyser.frequencyBinCount;
        const dataArray = new Uint8Array(bufferLength);

        const draw = () => {
            if (!this.isVisualizerRunning) return;
            requestAnimationFrame(draw);
            this.analyser.getByteFrequencyData(dataArray);

            this.canvasCtx.clearRect(0, 0, this.canvas.width, this.canvas.height);

            const barWidth = (this.canvas.width / bufferLength) * 2;
            let x = 0;

            for (let i = 0; i < bufferLength; i++) {
                const barHeight = (dataArray[i] / 255) * this.canvas.height;

                const grad = this.canvasCtx.createLinearGradient(0, this.canvas.height, 0, 0);
                grad.addColorStop(0, '#6366f1');
                grad.addColorStop(1, '#a855f7');

                this.canvasCtx.fillStyle = grad;
                this.canvasCtx.roundRect(x, this.canvas.height - barHeight, barWidth - 4, barHeight, [4, 4, 0, 0]);
                this.canvasCtx.fill();

                x += barWidth;
            }
        };
        draw();
    }

    setVolume(value) {
        this.currentVolume = parseFloat(value);
        if (this.workletNode) {
            this.workletNode.port.postMessage({ type: 'SET_VOLUME', volume: this.currentVolume });
        }
    }

    checkAudioState() {
        this.unlockOverlay.style.display = (this.audioCtx?.state === 'suspended') ? 'flex' : 'none';
    }

    unlockAudio() {
        this.audioCtx?.resume();
    }

    disconnectedCallback() {
        this.eventSource?.close();
        this._releaseLock?.();
        this.worker?.terminate();
        this.audioCtx?.close();
    }
}

customElements.define('radio-player', RadioPlayer);
