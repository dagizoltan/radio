# Hono SSR (main.tsx)

The `main.tsx` file is the entry point for the Deno application. It uses the Hono framework and its JSX engine to provide Server-Side Rendering (SSR) and API proxying.

## Server-Side Rendering

When a client makes a `GET /` request, the server executes the following sequence:

1.  **Manifest Fetch:** It fetches `live/manifest.json` from the configured R2 bucket (via the `R2_PUBLIC_URL` environment variable).
2.  **HTML Shell Render:** It uses Hono JSX to render the complete HTML `<html>` document.
    *   It includes the `<head>` with links to `/static/style.css`.
    *   It renders the main site header, including a live status badge (green if `live: true` in the manifest, grey if offline or unreachable).
3.  **Custom Element Injection:** It injects the `<radio-player>` custom element into the body.
    *   **Data Attributes:** Crucially, it attaches the state fetched from the manifest directly to the element as `data-*` attributes (e.g., `data-live="true"`, `data-latest="42"`, `data-duration="10"`).
4.  **Async Script Loading:** It includes `<script type="module" src="/static/player.js" async></script>`. The `async` attribute ensures the script never blocks the parsing and rendering of the HTML shell.

**Guarantee:** The SSR shell is complete HTML. A user with JavaScript disabled sees a proper, styled page reflecting the real-time status of the stream, even though playback requires JavaScript.

## Proxy Routes

The server acts as a proxy to the S3 bucket to avoid exposing the bucket directly and to manage CORS and caching headers.

### `GET /manifest.json`

*   **Action:** Fetches `live/manifest.json` from `R2_PUBLIC_URL`.
*   **Response:** Returns the JSON body.
*   **Headers:** Includes aggressive `Cache-Control: no-cache, no-store, must-revalidate` headers to ensure clients always get the freshest live edge.

### `GET /segment/:id`

*   **Action:** Formats the `:id` (e.g., `42`) into the segment key format (`live/segment-000042.flac`) and fetches it from `R2_PUBLIC_URL`.
*   **Response:** Returns the raw bytes as `audio/flac`.
*   **Headers:** Since segments are immutable once written, it includes aggressive caching headers (e.g., `Cache-Control: public, max-age=31536000, immutable`).

### `GET /static/:file`

*   **Action:** Serves static assets (`style.css`, `player.js`, `worklet.js`, `flac_decoder.js`, `flac_decoder_bg.wasm`) directly from the local `./static/` directory.

## Environment Variables

The server relies on two critical environment variables:

*   **`PORT`**: The HTTP port to bind to. Read from `Deno.env.get("PORT")`. Defaults to `3000`. It binds to `0.0.0.0` for Docker compatibility.
*   **`R2_PUBLIC_URL`**: The base URL for all S3 fetches. Read from `Deno.env.get("R2_PUBLIC_URL")`.
    *   In local development, this is `http://minio:9000/radio-stream`.
    *   In production, this is the public-facing R2 URL or a Cloudflare Worker URL.