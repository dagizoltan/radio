# Hono SSR (main.tsx)

The `main.tsx` file is the entry point for the Deno application. It uses the Hono framework and its JSX engine to provide Server-Side Rendering (SSR) and injects initial state for the client.

## Server-Side Rendering

When a client makes a `GET /` request, the server executes the following sequence:

1.  **Manifest Fetch:** It fetches `live/manifest.json` from the configured R2 bucket (via the `R2_PUBLIC_URL` environment variable).
2.  **HTML Shell Render:** It uses Hono JSX to render the complete HTML `<html>` document.
    *   It includes the `<head>` with links to `/static/style.css`.
    *   It renders the main site header, including a live status badge (green if `live: true` in the manifest, grey if offline or unreachable).
3.  **Custom Element Injection:** It injects the `<radio-player>` custom element into the body.
    *   **Data Attributes:** Crucially, it attaches the state fetched from the manifest directly to the element as `data-*` attributes (e.g., `data-live="true"`, `data-latest="42"`, `data-duration="10"`, `data-r2-url="${R2_PUBLIC_URL}"`).
4.  **Async Script Loading:** It includes `<script type="module" src="/static/player.js" async></script>`. The `async` attribute ensures the script never blocks the parsing and rendering of the HTML shell.

**Guarantee:** The SSR shell is complete HTML. A user with JavaScript disabled sees a proper, styled page reflecting the real-time status of the stream, even though playback requires JavaScript.

## The Edge Streaming Constraints

### Direct S3/R2 Fetch

**CRITICAL CONSTRAINT:** The Deno server must not proxy audio segments **or `manifest.json`**. Both are fetched directly from `R2_PUBLIC_URL` by the browser Web Component. The Deno server's sole post-SSR responsibility is serving static assets. Deno Deploy imposes strict egress bandwidth limits. The `radio-player` client-side script fetches the `.flac` and `.opus` chunks *directly* from the `R2_PUBLIC_URL` (Cloudflare CDN). This bypasses the Deno proxy entirely, transferring the massive bandwidth load of thousands of simultaneous streaming users completely onto Cloudflare R2's free-egress CDN.

*Requirement:* The S3/R2 bucket must have a strict CORS policy enabled (`Access-Control-Allow-Origin` matching the Deno Deploy URL) to allow the browser client to fetch these segments directly.

**No manifest proxy.** The Deno server does not proxy `manifest.json`. The `<radio-player>` Web Component fetches it directly from `${data-r2-url}/live/manifest.json`. Removing this proxy route eliminates the Deno server from the media-critical path.

### Manifest Fetch Failure Handling (SSR)

The server-side manifest fetch at `GET /` may fail if R2 is temporarily unreachable, the server has just started and no segments have been uploaded yet, or the bucket is misconfigured. The SSR must handle this gracefully rather than returning a 500 error.

**Behaviour on fetch failure:**
- Catch the error (network failure, non-200 response, JSON parse error).
- Render the HTML shell with fallback data attributes: `data-live="false"`, `data-latest="0"`, `data-duration="10"`, `data-r2-url="${R2_PUBLIC_URL}"`.
- The live status badge renders as grey/offline.
- The play button renders as disabled with the label "Stream Offline".

**Client recovery:** The `<radio-player>` Web Component initialises in polling-only mode. Its fetch loop retries the manifest (directly from R2 via `data-r2-url`) on a `segment_s`-second interval. When the manifest becomes available and `live: true`, the component automatically enables the play button and updates the live badge — without requiring a page reload.

**Implementation note:** The SSR manifest fetch must have a short timeout to avoid blocking page render during an R2 outage. Use `AbortSignal.timeout(3000)` in the Deno `fetch()` call. `AbortSignal.timeout()` is available in Deno 1.28+ and is supported by the pinned Docker image (`denoland/deno:2.0.0`). Always wrap the fetch in try/catch — do not let a timeout `AbortError` propagate as an unhandled rejection:

```typescript
let manifest: Manifest | null = null;
try {
  const res = await fetch(
    `${R2_PUBLIC_URL}/live/manifest.json`,
    { signal: AbortSignal.timeout(3000) }
  );
  if (res.ok) manifest = await res.json() as Manifest;
} catch (_err) {
  // Timeout, network error, or JSON parse failure — render offline fallback
}

const live = manifest?.live ?? false;
const latest = manifest?.latest ?? 0;
const duration = manifest?.segment_s ?? 10;
```

On any error path, the page renders with `data-live="false"` and the client recovers by polling R2 directly.

### `GET /static/:file`

*   **Action:** Serves static assets (`style.css`, `player.js`, `worklet.js`, `flac_decoder.js`, `flac_decoder_bg.wasm`, `opus_decoder.js`, `opus_decoder_bg.wasm`) directly from the local `./static/` directory.

### SSE Proxying Constraints

The Deno server must strictly rely on standard HTTP polling to fetch the `manifest.json`. It must **never** attempt to proxy the `/events` SSE stream from the Rust server to the public browser client. The SSE stream is exclusively for the local operator monitor UI (`localhost:8080`).

## Environment Variables

The server relies on two critical environment variables:

*   **`PORT`**: The HTTP port to bind to. Read from `Deno.env.get("PORT")`. Defaults to `3000`. It binds to `0.0.0.0` for Docker compatibility.
*   **`R2_PUBLIC_URL`**: Used for a single server-side manifest fetch at SSR render time to populate the live-status badge and initial `data-*` attributes on `<radio-player>`. It is also injected into the HTML as the `data-r2-url` attribute, after which the browser takes over all manifest polling and segment fetching directly against R2. The Deno server makes no further use of this URL after the initial render — it does not proxy any ongoing traffic.
    *   Local dev: `http://minio:9000/radio-stream`
    *   Production: the Cloudflare R2 public URL (e.g., `https://pub-xxxxxx.r2.dev/my-radio-stream`)