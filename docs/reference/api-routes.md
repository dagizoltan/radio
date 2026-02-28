# API Routes

This document catalogs all HTTP routes exposed by the system's two web servers.

## `radio-server` (Local Operator Monitor)

Served locally on `127.0.0.1:8080`.

| Method | Path | Description | Response Type | Caching |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/` | Serves the embedded `monitor.html` operator interface. | `text/html` | None |
| `GET` | `/events` | Streams Server-Sent Events (SSE) detailing system state. | `text/event-stream` | `no-cache` |
| `GET` | `/status` | Returns a point-in-time JSON snapshot of the `Arc<AppState>`. | `application/json` | None |
| `GET` | `/metrics` | Prometheus-format telemetry metrics. Exposes counters and gauges for capture overruns, normaliser gain, S3 PUT latency, and rolling window size. See [Observability Baseline](observability-baseline.md) for the full metric list and healthy ranges. | `text/plain` | `no-cache` |
| `POST` | `/start` | Sets the global `streaming` atomic boolean to `true`. | `text/plain` (`OK`) | None |
| `POST` | `/stop` | Sets the global `streaming` atomic boolean to `false`. | `text/plain` (`OK`) | None |
| `GET` | `/local/:id` | Returns the FLAC segment matching `:id` from the RAM rolling window. Prepends the cached FLAC stream header to make it playable. | `audio/flac` | `no-cache` |

## `radio-client` (Public Listener Frontend)

Served via Deno Deploy (or `localhost:3000` via Compose). Acts as the SSR frontend and manifest server.

| Method | Path | Description | Response Type | Caching |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/` | SSR Hono route. Fetches the manifest from R2 and renders the full HTML shell containing the `<radio-player>`. | `text/html` | None |
| `GET` | `/static/:file` | Serves JS, CSS, and WASM assets from the local `static/` directory (including `opus_decoder.js` and `opus_decoder_bg.wasm`). | varies (`text/css`, `application/javascript`, `application/wasm`) | Standard static asset caching |
> The manifest is fetched directly from `R2_PUBLIC_URL/live/manifest.json` by the browser. No Deno proxy route exists for it.
