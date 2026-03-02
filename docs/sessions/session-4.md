# Prompt for Session 4: Deno SSR and WASM Toolchain (The Bridge)

**Goal:** Set up the `radio-client` Deno server, the `/api/manifest` caching proxy, the HMAC token injection, and build the Rust-to-WASM decoders.

**Context & Requirements:**
You are shifting from the Rust backend to the Deno frontend and the browser WASM decoders. Note: The entire client-side and Deno codebase must be written in pure standard JavaScript (`.js`), NOT TypeScript or JSX. Use `hono/html` tagged template literals instead of JSX if needed.

**1. Hono SSR Server (`main.js`):**
- **SSR Route (`/`):** Use `import { html } from 'hono/html'` to render the HTML shell. Inject the `<radio-player>` custom element. Use pure JS template literals, no JSX transpilation step.
- **HMAC Token Injection:** Use the Web Crypto API (`crypto.subtle.sign`). If `Deno.env.get("TOKEN_SECRET")` is set, generate an HMAC SHA-256 token signing `Math.floor(Date.now() / 3600000)` (rotating hourly) mixed with the client's IP (`c.req.header('cf-connecting-ip')`). Inject it into `<radio-player data-token="xyz">`.
- **Manifest Proxy (`/api/manifest`):** Fetch `manifest.json` from the `R2_PUBLIC_URL`. Pass through the response exactly but set `Cache-Control: s-maxage=5, stale-while-revalidate=2` to coalesce client polls at the CDN edge. Support `If-None-Match` by passing the `ETag`.
- **Token Refresh (`/api/token`):** Implement a `POST` endpoint that generates and returns a fresh short-lived token (`{ "token": "xyz" }`) for clients recovering from a `403 Forbidden` response. Check `Origin` to require same-site.
- **Static Assets (`/static/*`):** Use `serveStatic` from `@hono/node-server/serve-static` (or Deno equivalent) to serve the JS/CSS and `.wasm` files.

**2. WASM Decoder (`decoder/flac`):**
- **Single Decoder Architecture:** Since both HQ and LQ streams are now FLAC, build only one WASM module. It must implement a `push(bytes: &[u8]) -> *const f32` API. Expose a `len() -> usize` method to get the length of the written buffer.
- **FLAC Decoder (`decoder/flac`):**
  - Implement the minimal verbatim subset. It must read the `STREAMINFO` block to dynamically learn the stream's sample rate and bps (e.g., 48kHz/24-bit or 24kHz/16-bit).
  - Implement tentative parse with rollback: `let start = reader.position();`. Parse frame header. `if remaining_bytes < required { reader.set_position(start); return; }`
  - Implement sign extension based on bps: `let sample: i32 = (raw_value as i32) << (32 - bps) >> (32 - bps);`. Normalize to `f32` (`sample as f32 / (1 << (bps - 1)) as f32`).
  - Provide a `reset()` method: clears the accumulator and resets the `header_parsed` boolean so the next push correctly expects the 38-byte stream header.
- **Zero-Copy Memory View:** In Javascript, after calling `wasm.push()`, do:
  `const pcmView = new Float32Array(wasm.memory.buffer, wasm.get_ptr(), wasm.get_len());`
  Do NOT call `postMessage(pcmView, [pcmView.buffer])`.

**Validation:**
Ensure the Deno server starts and correctly proxies the manifest with caching headers. Write a generic JS unit test that pushes raw 48kHz/24-bit and 24kHz/16-bit FLAC segment bytes (downloaded from the MinIO server) through the compiled WASM module and verifies that correct, float-normalized `[-1.0, 1.0]` PCM arrays are returned for both qualities.
