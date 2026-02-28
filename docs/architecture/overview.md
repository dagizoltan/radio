# Architecture Overview

The Lossless Vinyl Radio Streaming System is a two-part architecture designed to capture analog audio, encode it losslessly to FLAC, archive it locally, and stream it to listeners worldwide. The primary goal is achieving production-grade, high-quality (HQ) audio delivery, prioritizing stream stability and flawless playback over ultra-low latency.

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
|  | radio-server (Rust - 3 Processes)                   |  |
|  |                                                     |  |
|  |  1. [ HQ Recorder ] -> [ Local Archive ]            |  |
|  |           |                                         |  |
|  |           v                                         |  |
|  |  2. [ Converter (Norm, HQ & LQ Encode) ]            |  |
|  |           |                                         |  |
|  |           v                                         |  |
|  |  3. [ Cloud Uploader ] -> [ R2 Storage ]            |  |
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

## Signal Flow End-to-End

1. **Analog Capture:** The analog mixer's REC OUT is connected to inputs 1 and 2 of the Behringer UMC404HD USB audio interface.
2. **ALSA Interface:** The `radio-server` captures the audio directly from the Linux kernel ALSA interface using raw ioctls.
3. **Process 1: HQ Recorder:** The raw PCM audio is captured and encoded into high-quality FLAC verbatim subframes. It is saved directly to a local `./recordings` directory. This process isolates the archival bit-perfect copy.
4. **Process 2: Converter:** A separate converter process consumes the raw audio, passes it through a [two-stage normalizer](../radio-server/normalizer.md) (LUFS gain rider and true-peak limiter), and encodes it into multiple qualities (e.g., HQ FLAC and an LQ variant for lower bandwidth).
5. **Process 3: Cloud Uploader:** The multi-quality segments and a `manifest.json` are uploaded to an S3-compatible storage backend ([Cloudflare R2 in production, MinIO locally](../deployment/docker-compose.md)). The uploader manages the rolling window, supplemented by S3 Object Lifecycle Rules for robust cleanup.
6. **Listener Client:** The listener visits the Deno Deploy frontend (`radio-client`). The frontend performs a single server-side manifest fetch from R2 to populate the initial live-status badge and inject `data-*` attributes. It serves the HTML shell and static assets (JS, WASM). All subsequent manifest polling and segment fetching is performed browser-side directly against R2.
7. **Browser Playback:** The browser loads a Web Component island that continuously polls the manifest and fetches audio segments *directly* from the Cloudflare R2 edge CDN (bypassing the Deno proxy to save bandwidth). The segments are decoded in the browser using a [WASM FLAC decoder](../radio-client/wasm-decoder.md) for HQ segments or a [WASM Opus decoder](../radio-client/wasm-decoder.md) for LQ segments (selected automatically based on quality setting), and played via an [AudioWorklet](../radio-client/worklet.md).

## Two-Codebase Split

The system is strictly divided into two independent codebases:

1. **`radio-server`:** A pure Rust codebase running locally on the ThinkPad. It is structurally split into three main processes: one for capturing/archiving HQ audio, one for converting the stream to multi-quality (LQ) chunks, and one for uploading chunks to the cloud. It also serves a local operator monitor UI.
2. **`radio-client`:** A Deno + Hono application deployed to Deno Deploy. It serves the public listener interface, fetches the manifest from R2 **once per page request at SSR time** to populate initial state attributes, and provides the Web Component and WASM decoder for browser playback. All ongoing manifest polling and segment fetching is performed browser-side directly against R2.

## Docker Topology

For local development, the entire system is orchestrated using Docker Compose. The topology consists of four services:

*   **`minio`:** Provides S3-compatible storage, standing in for Cloudflare R2.
*   **`minio-setup`:** A temporary container that configures the `minio` bucket and policies.
*   **`radio`:** The Rust server capturing audio, encoding it, and uploading to `minio`.
*   **`client`:** The Deno frontend, serving the web listener interface and manifest.

See the [Docker Compose Documentation](../deployment/docker-compose.md) for full details.