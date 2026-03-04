import { LosslessDecoder } from './decoder.js';

let decoder = new LosslessDecoder();
let isInitialized = false;
let isPlaying = false;
let quality = 'hq'; // 'hq' or 'lq'
let token = null;
let r2Url = null;

let currentIndex = 0;
let latestIndex = 0;
let bufferTarget = 2; // Default pre-roll: latest - 2
let segmentLengthSec = 5;
let bandwidthEma = null; // Exponential Moving Average of bandwidth
const EMA_ALPHA = 0.3; // Weight of the new measurement (0.0 to 1.0)

// Buffer Pool for Zero-Copy transfers
const pool = [];

let workletPort = null;

onmessage = async (e) => {
    const msg = e.data;
    switch (msg.type) {
        case 'INIT':
            if (!isInitialized) {
                await decoder.init();
                isInitialized = true;
                token = msg.token;
                r2Url = msg.r2Url;
                postMessage({ type: 'INIT_DONE' });
                pollManifest();
            }
            break;
        case 'PLAY':
            workletPort = msg.port;
            isPlaying = true;
            break;
        case 'STOP':
            isPlaying = false;
            break;
        case 'SET_QUALITY':
            if (quality !== msg.quality) {
                quality = msg.quality;
                decoder.reset();
                if (workletPort) {
                    workletPort.postMessage('FLUSH');
                }
                currentIndex = latestIndex - bufferTarget; // Jump to new buffer target
                if (isPlaying) {
                    fetchNextSegment();
                }
            }
            break;
        case 'TOKEN_UPDATE':
            token = msg.token;
            break;
    }
};

async function refreshToken() {
    try {
        const res = await fetch('/api/token', { method: 'POST' });
        if (res.ok) {
            const data = await res.json();
            token = data.token;
            postMessage({ type: 'TOKEN_UPDATED', token });
        }
    } catch (err) {
        console.error("Token refresh failed:", err);
    }
}

async function pollManifest() {
    try {
        const res = await fetch('/api/manifest');
        if (res.ok) {
            const manifest = await res.json();
            latestIndex = manifest.latest_sequence || manifest.latest;

            if (manifest.updated_at && Date.now() - manifest.updated_at > segmentLengthSec * 3 * 1000) {
                postMessage({ type: 'STALE_MANIFEST', isStale: true });
            } else {
                postMessage({ type: 'STALE_MANIFEST', isStale: false });
            }

            // Initial sync or jump-ahead if too far behind
            if (currentIndex === 0 || currentIndex < latestIndex - 6) {
                currentIndex = latestIndex - bufferTarget;
            }
        }
    } catch (err) {
        console.error("Failed to fetch manifest:", err);
    }

    if (isPlaying) {
        fetchNextSegment();
    } else {
        setTimeout(pollManifest, segmentLengthSec * 1000);
    }
}

async function fetchNextSegment() {
    if (!isPlaying) return;

    // Jump-Ahead / Rollover Logic
    // If we are way behind or sequence wrapped around (e.g., rollover at 100,000,000)
    if (latestIndex < currentIndex && currentIndex - latestIndex > 3) {
        console.log(`Rollover detected: currentIndex ${currentIndex}, latestIndex ${latestIndex}`);
        currentIndex = latestIndex;
    } else if (currentIndex < latestIndex - bufferTarget - 1) {
        console.log(`Jumping ahead from ${currentIndex} to ${latestIndex - 1}`);
        currentIndex = latestIndex - 1;
    }

    // Fix index constraint #14: 8-digit padding
    const paddedIndex = currentIndex.toString().padStart(8, '0');

    // Fetch directly from R2 URL (Constraint #11, #13)
    const segmentUrl = `${r2Url}/live/${quality}/segment-${paddedIndex}.flac?token=${token}`;
    const startTime = performance.now();

    try {
        const response = await fetch(segmentUrl);

        if (response.status === 403) {
            console.log("403 Forbidden, refreshing token...");
            await refreshToken();
            setTimeout(fetchNextSegment, 500); // Retry after refresh
            return;
        }

        if (response.status === 404) {
            console.log(`404 Not Found: ${currentIndex}, repolling manifest...`);
            // 404 Infinite Loop Protection: Sleep for at least segment_s / 2
            setTimeout(pollManifest, (segmentLengthSec / 2) * 1000);
            return;
        }

        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }

        const arrayBuffer = await response.arrayBuffer();
        const endTime = performance.now();

        // Dynamic Buffering Strategy
        const bytes_downloaded = arrayBuffer.byteLength;
        const time_taken_ms = endTime - startTime;

        if (time_taken_ms > 0) {
            const bandwidth_bps = (bytes_downloaded * 8) / (time_taken_ms / 1000);

            // Calculate Exponential Moving Average
            if (bandwidthEma === null) {
                bandwidthEma = bandwidth_bps;
            } else {
                bandwidthEma = (EMA_ALPHA * bandwidth_bps) + ((1 - EMA_ALPHA) * bandwidthEma);
            }

            // Report metrics for monitoring
            postMessage({ type: 'METRICS', bandwidth_bps: bandwidthEma, time_taken_ms });

            // Adjust pre-roll target based on bandwidth EMA
            if (quality === 'hq' && bandwidthEma < 1500000) {
                bufferTarget = 4;
            } else if (quality === 'lq' && bandwidthEma < 800000) {
                bufferTarget = 4;
            } else {
                bufferTarget = 2; // Default
            }
        }

        // Decode using WASM
        let pcm;
        try {
            pcm = decoder.decode(new Uint8Array(arrayBuffer));
        } catch (decodeErr) {
            console.error(`Error decoding segment ${currentIndex}:`, decodeErr);
            currentIndex++;
            const fetchDuration = endTime - startTime;
            const timeToWait = Math.max(0, (segmentLengthSec * 1000) - fetchDuration - 500);
            setTimeout(fetchNextSegment, timeToWait);
            return;
        }

        // Zero-Copy Buffer Pool Transfer
        if (pcm.length > 0 && workletPort) {
            const pcmCopy = pool.pop() || new Float32Array(pcm.length);
            if (pcmCopy.length !== pcm.length) {
                // If pool gives different size, just create new
                const newCopy = new Float32Array(pcm.length);
                newCopy.set(pcm);
                workletPort.postMessage(newCopy, [newCopy.buffer]);
            } else {
                pcmCopy.set(pcm);
                workletPort.postMessage(pcmCopy, [pcmCopy.buffer]);
            }
        }

        currentIndex++;

        // Calculate time to wait before fetching next
        const fetchDuration = endTime - startTime;
        const timeToWait = Math.max(0, (segmentLengthSec * 1000) - fetchDuration - 500); // 500ms safety buffer

        setTimeout(fetchNextSegment, timeToWait);

    } catch (err) {
        console.error(`Error fetching segment ${currentIndex}:`, err);
        // On generic error, repoll manifest after delay
        setTimeout(pollManifest, (segmentLengthSec / 2) * 1000);
    }
}
