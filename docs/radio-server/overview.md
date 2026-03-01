# Radio Server Overview

The `radio-server` is a pure Rust binary running on the ThinkPad. It is responsible for capturing audio, encoding it to FLAC, archiving it locally, uploading segments to R2, and serving a local monitor UI.

## Crate Workspace Layout

The server is organized as a Cargo workspace containing three crates:

*   **`crates/capture`**: Direct ALSA audio capture via raw ioctls. Compiled as an `rlib`.
*   **`crates/encoder`**: Pure Rust FLAC encoder generating verbatim subframes. Compiled as an `rlib`.
*   **`crates/server`**: The main binary. Depends on `capture` and `encoder`. It orchestrates the asynchronous tasks and manages shared state.

## Dependency Graph

```text
[crates/server]
  |
  +--> [crates/capture] (Uses rustix)
  |
  +--> [crates/encoder]
```

## Task Orchestration & Error Handling

The main binary runs three distinct primary processes (logically separated, though orchestrated concurrently in Tokio via `tokio::select!` in `main`).

**Error Handling Strategy:** All processes must employ a robust error-handling strategy (e.g., using `anyhow` for application-level errors or a custom `thiserror` enum for library crates). A transient failure (such as an `EWOULDBLOCK` from ALSA or a temporary network drop in the S3 uploader) must be logged and retried. A fatal failure in one task must signal the cancellation token to gracefully tear down the other tasks via the `tokio::select!` macro, flushing buffers and closing file handles before the process exits.

1.  **Process 1: HQ Recorder Task**: Reads directly from the ALSA capture device, encodes raw high-quality (HQ) FLAC verbatim frames, writes them to the local archive disk, and sends the raw PCM periods to the Converter Task via a bounded `mpsc` channel (capacity 16).
2.  **Process 2: Converter Task**: Receives the raw PCM periods, encodes them directly into multiple qualities (e.g., HQ FLAC and an LQ Opus version). It assembles these into complete 10-second segments and sends the completed segments to the Cloud Uploader Task via a bounded `mpsc` channel (capacity 3).
3.  **Process 3: Cloud Uploader Task**: Receives the completed HQ and LQ segments, manages the rolling window queue, uploads all segment files to S3, and updates the multi-quality stream `manifest.json`.

*(An additional **HTTP Task** runs concurrently to serve the local operator monitor UI on `127.0.0.1:8080` and manage SSE connections.)*

## Shared State

These tasks coordinate via an `Arc<AppState>` containing:

*   `streaming: AtomicBool`: Global streaming status.
*   `vu_left, vu_right: AtomicI32`: Peak sample values for the UI.
*   `recording_path: Mutex<String>`: Current local archive file path.
*   `recording_bytes: AtomicU64`: Total bytes written to the current archive.
*   `r2_segment: AtomicU64`: Current segment index.
*   `r2_last_ms: AtomicU64`: Timestamp of the last successful upload.
*   `r2_uploading: AtomicBool`: True while an upload is in progress.
*   `local_segments: Mutex<VecDeque<(u64, Bytes)>>`: The last 3 segments kept in RAM for local monitor playback.
*   `flac_header: Mutex<Option<Bytes>>`: Cached FLAC stream header (prepended to segments for local playback).
*   `sse_tx: broadcast::Sender<String>`: The SSE event bus for broadcasting state changes to the monitor UI.