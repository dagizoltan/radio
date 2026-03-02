# Prompt for Session 4: Deno SSR and WASM Toolchain (The Bridge)

**Goal:** Set up the `radio-client` Deno server, the `/api/manifest` caching proxy, the HMAC token injection, and build the Rust-to-WASM decoders.

**Context & Requirements:**
You are shifting from the Rust backend to the Deno frontend and the browser WASM decoders.

**1. Hono SSR Server (`main.tsx`):**
- **SSR Route (`/`):** Render the HTML shell containing the `<radio-player>` custom element.
- **HMAC Token Injection:** If `TOKEN_SECRET` is set in the environment, generate a short-lived cryptographic HMAC token (incorporating a simple session ID or client IP if available) and inject it into the `<radio-player>` via the `data-token` attribute.
- **Manifest Proxy (`/api/manifest`):** Fetch `manifest.json` from the `R2_PUBLIC_URL`. Set `Cache-Control: s-maxage=5` on the response to leverage CDN edge caching and dramatically reduce R2 Class B GET costs. Pass through the `ETag` from R2.
- **Token Refresh (`/api/token`):** Implement a `POST` (or `GET`) endpoint that generates and returns a fresh short-lived token for clients recovering from a `403 Forbidden` response without needing a hard page reload. Require same-origin.
- **Static Assets (`/static/*`):** Serve compiled JS, CSS, and WASM files.

**2. WASM Decoders (`decoder/flac` & `decoder/opus`):**
- **Opus Decoder (`decoder/opus`):**
  - Use `opus-rs` to decode raw packets to `f32` PCM.
  - Implement a `push(bytes: &[u8])` API that reads the 2-byte Big Endian payload length prefix, extracts the packet, and decodes it continuously.
  - Add a safety check: `if buffer.len() < 2`, wait for the next chunk before reading the prefix.
- **FLAC Decoder (`decoder/flac`):**
  - Implement the minimal subset decoder (verbatim subframes, fixed block size/rate codes, 24-bit).
  - Implement the variable-length frame header tentative parse with **rollback**: record offset, parse header, calculate required bytes (header + samples + CRC), and if insufficient, reset the read position and return.
  - **Zero-Copy Optimization:** Expose the internal `f32` buffer directly to JS to avoid allocation overhead. Do NOT allocate a new `Vec<f32>` on every chunk.
  - Provide a `decoder.reset()` API to parse the `fLaC` stream header cleanly across segment boundaries.
- **Compilation:** Ensure `wasm-pack build --target web` successfully compiles both. (Note: Opus will require Emscripten/wasi-sdk for the libopus C dependency).

**Validation:**
Ensure the Deno server starts and correctly proxies the manifest with caching headers. Write a generic JS unit test that pushes raw FLAC and Opus segment bytes (downloaded from the MinIO server) through the compiled WASM modules and verifies that correct, float-normalized `[-1.0, 1.0]` PCM arrays are returned.
