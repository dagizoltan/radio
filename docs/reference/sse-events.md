# SSE Events

This document details the Server-Sent Events (SSE) emitted by the `radio-server` on the `/events` endpoint, consumed by the [Monitor UI](../radio-server/monitor-ui.md).

All events are broadcast via the `sse_tx` channel (`tokio::sync::broadcast::Sender<String>`). The payload is always a JSON string.

| Event Type Name | Emitting Task | Consuming UI Component | Payload Schema (JSON) | Description |
| :--- | :--- | :--- | :--- | :--- |
| `status` | HTTP Task (`/start`, `/stop`) | Header Live Pill, Controls Row (Start/Stop Buttons) | `{"live": boolean}` | Indicates if the system is actively capturing and uploading. |
| `vu` | Pipeline Task (every 50ms) | Dual VU Meters | `{"left": number, "right": number}` | Peak absolute sample values (`0-32768`) for the last 50ms block. |
| `gain` | Pipeline Task (every 50ms) | Dual VU Meters (Gain Rider Display) | `{"gain_db": number}` | The current smoothed LUFS gain adjustment applied by the normalizer, in dB. |
| `recording` | Recorder Task (every 1s) | Local Recording Card | `{"path": string, "bytes": number}` | The current local archive file path and its size in bytes. |
| `r2` | R2 Uploader Task | R2 Segment Timeline Pills | `{"uploading": boolean, "segment": number, "last_ms": number}` | Status of the S3 upload rolling window. `uploading` is true during an HTTP PUT. `segment` is the current index. `last_ms` is the timestamp of the last success. |