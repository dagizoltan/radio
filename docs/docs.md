# Lossless Vinyl Radio Streaming System Documentation

This system captures analog audio from a vinyl turntable (via a Behringer UMC404HD USB interface), encodes it losslessly to FLAC, archives a pristine copy locally on a ThinkPad, encodes the stream to multiple qualities, and pushes 10-second segments to Cloudflare R2 (or local MinIO). Listeners worldwide consume the live stream through a Deno Deploy frontend that decodes the FLAC chunks directly in the browser using a custom WASM decoder and an AudioWorklet.

## System Diagram

```text
[ Analog Source ]
       |
       v (REC OUT)
[ UMC404HD USB Interface ]
       |
       v (USB / ALSA)
+-----------------------------------------------------------+
| Lenovo ThinkPad X270 (Ubuntu Linux)                       |
|                                                           |
|  +-----------------------------------------------------+  |
|  | radio-server (Rust)                                 |  |
|  |                                                     |  |
|  |  [ Capture ] -> [ Raw Encoder ] -> [ Local Archive ]|  |
|  |       |                                             |  |
|  |       v                                             |  |
|  |  [ Converter (HQ & LQ Encode) ] -> [ R2 Uploader ]  |  |
|  |                                                     |  |
|  |  [ HTTP/SSE Monitor UI ]                            |  |
|  +-----------------------------------------------------+  |
|           |                                               |
|           v (S3 API)                                      |
|  [ Docker: MinIO (Development) ]                          |
+-----------------------------------------------------------+
       | (Production: Cloudflare R2)
       v
[ Cloudflare R2 ] (Live FLAC segments & Manifest)
       |
       v
+-----------------------------------------------------------+
| Deno Deploy                                               |
|                                                           |
|  +-----------------------------------------------------+  |
|  | radio-client (Deno + Hono)                          |  |
|  |                                                     |  |
|  |  [ SSR Frontend ]  <-- Fetches Manifest (SSR only)  |  |
|  +-----------------------------------------------------+  |
+-----------------------------------------------------------+
       | (Serves HTML, Manifest, JS/WASM)
       v
[ Browser Listener ] <-- Fetches Segments directly from R2
   |
   +-- <radio-player> Web Component
   +-- WASM FLAC Decoder (HQ)
   +-- WASM Opus Decoder (LQ)
   +-- AudioWorklet
```

## Quick Start (local dev)

```bash
git clone <repository_url> radio-stream
cd radio-stream/radio-server
echo "MINIO_USER=minioadmin\nMINIO_PASS=minioadmin123\nR2_BUCKET=radio-stream" > .env
docker compose up --build
```
*   Operator Monitor: `http://localhost:8080`
*   Listener Interface: `http://localhost:3000`
*   MinIO Console: `http://localhost:9001`

## Repository Layout

The project consists of two codebases and a Docker Compose orchestration setup:

```text
radio-stream/
├── radio-server/
│   ├── Cargo.toml          (Workspace root)
│   ├── docker-compose.yml  (Local dev topology)
│   ├── .env
│   ├── .env.prod
│   ├── crates/
│   │   ├── capture/
│   │   ├── encoder/
│   │   └── server/
│   └── Dockerfile
└── radio-client/
    ├── deno.json
    ├── main.tsx            (Hono entry point)
    ├── islands/
    │   ├── player.ts
    │   └── worklet.ts
    ├── decoder/
    │   ├── flac/           (Rust WASM FLAC decoder crate)
    │   └── opus/           (Rust WASM Opus decoder crate)
    ├── static/
    └── Dockerfile
```

## Table of Contents

### Architecture
*   [Overview](architecture/overview.md): High-level system diagram and end-to-end signal flow.
*   [Data Flow](architecture/data-flow.md): The lifecycle of audio samples, including the critical two-channel pipeline and segment lifecycle.
*   [Design Decisions](architecture/decisions.md): The rationale behind avoiding `libasound`, choosing verbatim FLAC, using `AudioWorklet`, and implementing a rolling window.

### Radio Server (Rust)
*   [Overview](radio-server/overview.md): Crate workspace layout, dependency graph, and shared `AppState`.
*   [Capture Crate](radio-server/capture.md): Pure Rust ALSA capture via `rustix` ioctls and `AsyncFd`.
*   [Encoder Crate](radio-server/encoder.md): Verbatim FLAC encoder, frame structure, and `BitWriter`.
*   [Server Crate](radio-server/server.md): Tokio task orchestration (Pipeline, Recorder, R2 Uploader, HTTP).
*   [AWS Signature V4 & Cloud Uploading](radio-server/aws-sig-v4.md): Transitioning to robust standard crates like `object_store` from the custom SigV4 implementation.
*   [Monitor UI](radio-server/monitor-ui.md): The embedded local operator interface (HTML/CSS/JS).

### Radio Client (Deno)
*   [Overview](radio-client/overview.md): Islands architecture connecting SSR to the Web Component and WASM decoder.
*   [Hono SSR](radio-client/hono-ssr.md): `main.tsx` routing, HTML shell rendering, and manifest fetching.
*   [Player Component](radio-client/player-component.md): The `<radio-player>` Web Component and chunk-fetching loop.
*   [AudioWorklet](radio-client/worklet.md): The browser audio thread processor handling PCM queues and playback.
*   [WASM Decoder](radio-client/wasm-decoder.md): The Rust-to-WASM crate that parses verbatim FLAC chunks into `f32` PCM.
*   [Styling](radio-client/styling.md): CSS custom properties, layout, and animations for the radio aesthetic.

### Deployment
*   [Docker Compose](deployment/docker-compose.md): The four-service topology (`minio`, `minio-setup`, `radio`, `client`).
*   [Local Development](deployment/local-dev.md): Step-by-step guide for running the system locally.
*   [Production](deployment/production.md): Deploying the server to Cloudflare R2 and the client to Deno Deploy.

### Reference
*   [Environment Variables](reference/environment-variables.md): Complete list of all configurable `.env` values.
*   [SSE Events](reference/sse-events.md): Schema for all Server-Sent Events emitted to the monitor UI.
*   [FLAC Format Subset](reference/flac-format.md): Detail of the specific verbatim FLAC encoding used.
*   [ALSA ioctls](reference/alsa-ioctls.md): Raw hex constants and `#[repr(C)]` structs for kernel interaction.
*   [API Routes](reference/api-routes.md): Exhaustive list of all HTTP endpoints across both servers.
*   [Manifest Schema](reference/manifest-schema.md): Field-by-field specification of `manifest.json`.
*   [Archive Integrity](reference/archive-integrity.md): Verification procedures for local FLAC recordings.
*   [Observability Baseline](reference/observability-baseline.md): Healthy metric ranges and alarm thresholds.

## Critical Constraints

The system design enforces several strict rules outlined below. Violating these constraints breaks the core archival and performance guarantees of the system.

1.  **No C bindings in capture**: The [Capture Crate](radio-server/capture.md) must not link against `libasound`. All interaction is via raw kernel ioctls.
2.  **Separate channels for audio and control**: The Server Pipeline uses two bounded `tokio::sync::mpsc` channels for audio data (raw PCM from Recorder to Converter, and assembled segments from Converter to Uploader). A single `tokio::sync::broadcast` channel is used exclusively for the SSE event bus.
3.  **Rolling window, not TTL**: R2 does not have reliable instant TTL. The [R2 Uploader Task](radio-server/server.md) maintains exactly 10 segments, actively deleting older keys.
4.  **Segments are complete FLAC files**: Every segment uploaded to R2 must contain the full FLAC stream header and be playable as a standalone file, as described in the [Encoder Spec](radio-server/encoder.md).
5.  **WASM decoder is minimal**: The [WASM Decoder](radio-client/wasm-decoder.md) only parses verbatim subframes matching our specific block size/rate encoding (24-bit). It does not implement full FLAC.
6.  **AudioWorklet for audio output**: The browser player must use an [AudioWorklet](radio-client/worklet.md) running on a dedicated thread, not the deprecated `ScriptProcessorNode`.
7.  **Monitor is embedded**: The [Monitor UI](radio-server/monitor-ui.md) is a single, dependency-free HTML file compiled into the Rust binary via `include_str!()`.
8.  **Path-style S3 URLs**: All S3 uploads via [AWS Sig V4](radio-server/aws-sig-v4.md) must use path-style URLs to ensure compatibility between MinIO and Cloudflare R2.
9.  **ALSA device discovery is dynamic**: The [Capture Crate](radio-server/capture.md) finds the UMC404HD card number at runtime by parsing `/proc/asound/cards`.
10. **Tokio AsyncFd for capture, not threads**: The capture loop must use `AsyncFd` for kernel-driven wakeups instead of a blocking polling loop, as detailed in the [Capture Crate](radio-server/capture.md) doc.
11. **No Proxying**: The public `radio-client` serves only the HTML shell and static assets. The browser Web Component must fetch audio segments *directly* from the S3/R2 bucket. The Deno server must **never proxy audio segments** or the `/events` SSE stream to the public internet to conserve bandwidth and connection limits.
12. **LQ stream is Opus-in-Ogg**: The LQ fallback stream uses the Opus codec at 128 kbps wrapped in an Ogg container. File extension is `.opus`. A dedicated `OpusDecoder` WASM module handles LQ decoding.
13. **No manifest proxy**: The browser fetches `manifest.json` and all segments directly from R2/MinIO using the `data-r2-url` SSR-injected attribute. The Deno server never proxies media traffic.
14. **8-digit segment indices**: All segment keys use 8-digit zero-padded indices (`segment-{:08}`). Index wraps at 100,000,000. Client handles rollover via sign-flip detection in jump-ahead logic.
15. **Single active tab**: The Web Locks API enforces one active decoder pipeline per user. A second tab shows a "Stream is already playing in another tab" message.