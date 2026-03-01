# SSE Events

This document details the Server-Sent Events (SSE) emitted by the `radio-server` on the `/events` endpoint, consumed by the [Monitor UI](../radio-server/monitor-ui.md).

All events are broadcast via the `sse_tx` channel (`tokio::sync::broadcast::Sender<String>`). The payload is always a JSON string.

| Event Type Name | Emitting Task | Consuming UI Component | Payload Schema (JSON) | Description |
| :--- | :--- | :--- | :--- | :--- |
| `status` | HTTP Task (`/start`, `/stop`) | Header Live Pill, Controls Row (Start/Stop Buttons) | `{"live": boolean}` | Indicates if the system is actively capturing and uploading. |
| `vu` | Recorder Task (per ALSA period, ~85ms at 4096 frames / 48000 Hz) | Dual VU Meters | `{"left": number, "right": number}` | Peak absolute sample values for the most recent ALSA capture period (~85ms). Values are raw 24-bit peak magnitudes in the range `0–8,388,607`. The monitor UI is responsible for scaling these to its display range. |
| `recording` | Recorder Task (every 1s) | Local Recording Card | `{"path": string, "bytes": number}` | The current local archive file path and its size in bytes. |
| `r2` | R2 Uploader Task | R2 Segment Timeline Pills | `{"uploading": boolean, "segment": number, "last_ms": number, "error": boolean}` | Status of the S3 upload rolling window. `uploading` is true during an HTTP PUT. `segment` is the current index. `last_ms` is the timestamp of the last success. `error` is `true` only when all retries for a segment have been exhausted and the segment was skipped; omitted or `false` on normal events. The monitor UI should display a visual error indicator on the affected segment pill when `error: true` is received. |