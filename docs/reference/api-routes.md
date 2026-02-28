# API Routes

This document catalogs all HTTP routes exposed by the system's two web servers.

## `radio-server` (Local Operator Monitor)

Served locally on `127.0.0.1:8080`.

| Method | Path | Description | Response Type | Caching |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/` | Serves the embedded `monitor.html` operator interface. | `text/html` | None |
| `GET` | `/events` | Streams Server-Sent Events (SSE) detailing system state. | `text/event-stream` | `no-cache` |
| `GET` | `/status` | Returns a point-in-time JSON snapshot of the `Arc<AppState>`. | `application/json` | None |
| `POST` | `/start` | Sets the global `streaming` atomic boolean to `true`. | `text/plain` (`OK`) | None |
| `POST` | `/stop` | Sets the global `streaming` atomic boolean to `false`. | `text/plain` (`OK`) | None |
| `GET` | `/local/:id` | Returns the FLAC segment matching `:id` from the RAM rolling window. Prepends the cached FLAC stream header to make it playable. | `audio/flac` | `no-cache` |

## `radio-client` (Public Listener Proxy)

Served via Deno Deploy (or `localhost:3000` via Compose). Acts as a proxy to S3/R2.

| Method | Path | Description | Response Type | Caching |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/` | SSR Hono route. Fetches the manifest from R2 and renders the full HTML shell containing the `<radio-player>`. | `text/html` | None |
| `GET` | `/manifest.json` | Proxies the `live/manifest.json` from the R2 bucket. The manifest now includes `qualities: ["hq", "lq"]`. | `application/json` | `no-cache, no-store, must-revalidate` (Critical for live edge discovery) |
| `GET` | `/segment/:quality/:id` | Proxies `live/{quality}/segment-{padded}.flac` from the R2 bucket based on user selection (`hq` or `lq`). | `audio/flac` | `public, max-age=31536000, immutable` (Segments are permanently immutable) |
| `GET` | `/static/:file` | Serves JS, CSS, and WASM assets from the local `static/` directory. | varies (`text/css`, `application/javascript`, `application/wasm`) | Standard static asset caching |