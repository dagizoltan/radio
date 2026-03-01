# Monitor UI

The operator monitor is a single-page web interface served by the `radio-server` at `http://127.0.0.1:8080`. It allows the operator to observe the capture levels, recording status, and R2 upload progress in real-time, right from the ThinkPad.

## Single File Embedding

The entire UI is contained within a single file: `crates/server/static/monitor.html`.

*   It contains all HTML, inline CSS (`<style>`), and inline JavaScript (`<script>`).
*   There are no external dependencies, no build steps, and no CDNs.
*   It is embedded directly into the Rust binary at compile time using the `include_str!()` macro.

## Layout and Styling

The UI utilizes a dark theme palette (`#0f0f0f` background) and a monospace font for a technical aesthetic.

*   **Header:** Shows the project logo and a `live/offline` status pill.
*   **Controls Row:** Full-width row containing Start/Stop buttons and an uptime counter.
*   **Grid Layout:** Below the controls is a 2-column grid.
    *   **Column 1:** Dual VU meter bars and the Local Audio Monitor.
    *   **Column 2:** Local Recording Card and R2 Segment Timeline.

## UI Components

### Dual VU Meters

Displays real-time audio levels for the Left and Right channels.
*   Data driven by the `vu` SSE event. Values are raw 24-bit peak magnitudes in the range `0–8,388,607`. The monitor UI must scale these to its display range (e.g., divide by 8,388,607 to get a `0.0–1.0` normalized level, or divide by 32,768 to approximate a 16-bit display scale).
*   Visual bars animate using CSS transforms or canvas rendering.
*   Color-coded zones: Green (normal), Yellow (approaching 0 dBFS), Red (clipping).
*   Includes a numeric dBFS readout.

### Local Audio Monitor

Allows the operator to listen to the live encoded stream directly from the server's RAM.
*   **Playback:** Uses `fetch()` to hit the `/local/:id` endpoint.
*   **Decoding:** Since this is a controlled environment (modern browser on localhost), it uses the built-in Web Audio API `AudioContext.decodeAudioData()`. It does not require the WASM decoder used by the public client.
*   **Sequencing:** Plays segments sequentially as they arrive.
*   **UI:** Play button, volume slider.
*   **Waveform:** An `AnalyserNode` connected to the `AudioContext` drives a `<canvas>` element. The waveform is drawn using `requestAnimationFrame`.

### Local Recording Card

Displays the status of the raw archive on the ThinkPad's local disk.
*   Data driven by the `recording` SSE event.
*   Displays the current full path to the FLAC file (`./recordings/...`).
*   Displays a continuously updating formatted file size (e.g., `45.2 MB`).

### R2 Segment Timeline

Visualizes the state of the rolling window uploads to S3.
*   Data driven by the `r2` SSE event.
*   Displays the last 10 segments as pill elements.
*   Pills indicate state: Uploading (animating), Active (in the window), or Deleted.
*   Shows the timestamp of the last successful upload.

## Server-Sent Events (SSE) Handling

The JavaScript connects to `/events` using an `EventSource`. It listens for various JSON event payloads to update the UI components reactively. See [SSE Events Reference](../reference/sse-events.md) for the exact schema of each event.

## Critical Constraints

**CRITICAL CONSTRAINT:** The monitor HTML file is embedded in the Rust binary via `include_str!()`. It is a single file with all CSS and JS inline.