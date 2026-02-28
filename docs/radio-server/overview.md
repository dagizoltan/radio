# Radio Server Overview

The `radio-server` is a pure Rust binary running on the ThinkPad. It is responsible for capturing audio, encoding it to FLAC, normalizing the signal, archiving it locally, uploading segments to R2, and serving a local monitor UI.

## Crate Workspace Layout

The server is organized as a Cargo workspace containing four crates:

*   **`crates/capture`**: Direct ALSA audio capture via raw ioctls. Compiled as an `rlib`.
*   **`crates/encoder`**: Pure Rust FLAC encoder generating verbatim subframes. Compiled as an `rlib`.
*   **`crates/normalizer`**: Two-stage audio normalization (LUFS gain rider + true-peak limiter). Compiled as an `rlib`.
*   **`crates/server`**: The main binary. Depends on `capture`, `encoder`, and `normalizer`. It orchestrates the asynchronous tasks and manages shared state.

## Dependency Graph

```text
[crates/server]
  |
  +--> [crates/capture] (Uses rustix)
  |
  +--> [crates/encoder]
  |
  +--> [crates/normalizer]
```

## Task Orchestration

The main binary uses Tokio to run four concurrent tasks via `tokio::select!` in `main`:

1.  **Pipeline Task**: Reads from the capture device, runs the normalizer, and broadcasts raw and normalized frames.
2.  **Recorder Task**: Receives raw frames and writes them directly to disk.
3.  **R2 Uploader Task**: Receives normalized frames, assembles 10-second segments, uploads them to S3, and updates the manifest.
4.  **HTTP Task**: Serves the local monitor UI (`monitor.html`) on `127.0.0.1:8080` and manages SSE connections.

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