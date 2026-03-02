# Prompt for Session 4: Deno SSR and WASM Toolchain (The Bridge)

**Goal:** Set up the `radio-client` Deno server, the caching proxy for the manifest, the pure Javascript frontend skeleton, and the Rust-to-WASM standalone FLAC decoder.

## 1. Deno Hono Server (`radio-client/main.js`)

You must write a pure standard JavaScript Deno server using the Hono framework. DO NOT use TypeScript or JSX.

### 1.1 Server Setup & SSR (`GET /`)
1. Import Hono and the HTML template literal tag:
   ```javascript
   import { Hono } from 'hono';
   import { html } from 'hono/html';
   import { serveStatic } from 'hono/deno';
   const app = new Hono();
   ```
2. In the root route `app.get('/', async (c) => { ... })`, fetch the manifest from `Deno.env.get("R2_PUBLIC_URL") + "/live/manifest.json"`.
3. If the fetch succeeds, parse it. If it fails, fallback to `live: false`.
4. Generate the HMAC token (see 1.2).
5. Return the rendered HTML using the `html` tagged template. Inject the `<radio-player>` element and pass the variables as `data-*` attributes:
   ```javascript
   return c.html(html`
     <!DOCTYPE html>
     <html lang="en">
     <head>
       <link rel="stylesheet" href="/static/style.css">
       <script type="module" src="/static/player.js" async></script>
     </head>
     <body>
       <radio-player
         data-r2-url="${r2Url}"
         data-live="${isLive}"
         data-latest="${latestIndex}"
         data-duration="${segmentS}"
         data-token="${token}"
       ></radio-player>
     </body>
     </html>
   `);
   ```

### 1.2 Token Generation (`GET /api/token` & Internal)
Implement a token generator using the standard Web Crypto API.
1. Read `Deno.env.get("TOKEN_SECRET")`. If missing, return an empty string.
2. Get the client IP from `c.req.header('cf-connecting-ip')` or default to `"unknown"`.
3. Calculate the current hour: `const hour = Math.floor(Date.now() / 3600000);`.
4. The message to sign is `hour + ":" + ip`.
5. Use `crypto.subtle.importKey` with `HMAC` and `SHA-256`.
6. Use `crypto.subtle.sign`. Convert the resulting ArrayBuffer to a hex string.
7. Expose this exact same logic on `app.post('/api/token', ...)` returning `{ "token": "hex_string" }`.

### 1.3 Manifest Proxy (`GET /api/manifest`)
1. Fetch `manifest.json` from R2.
2. Pass the response body through to the client.
3. Overwrite the caching headers to coalesce requests:
   ```javascript
   c.header('Cache-Control', 's-maxage=5, stale-while-revalidate=2');
   ```
4. If the R2 response contains an `ETag`, pass it through. If the client request contains `If-None-Match`, pass it to R2.

### 1.4 Static Assets
```javascript
app.use('/static/*', serveStatic({ root: './' }));
```

## 2. WASM FLAC Decoder (`decoder/flac`)

Create a Rust library crate compiled with `wasm-pack build --target web`.

### 2.1 Struct and State
```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct FlacDecoder {
    accumulator: Vec<u8>,
    output_pcm: Vec<f32>,
    header_parsed: bool,
    sample_rate: u32,
    channels: u8,
    bps: u8,
}
```

### 2.2 JavaScript API
Implement the `push` method to accept a chunk, decode all complete frames, and return a pointer to the WASM linear memory.
```rust
#[wasm_bindgen]
impl FlacDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self { ... }

    pub fn reset(&mut self) {
        self.accumulator.clear();
        self.output_pcm.clear();
        self.header_parsed = false;
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.output_pcm.clear();
        self.accumulator.extend_from_slice(chunk);

        // 1. If !header_parsed, search for 'fLaC'.
        // 2. Parse the STREAMINFO block to set self.sample_rate, self.channels, self.bps.
        // 3. Set header_parsed = true.

        // 4. Decode loop: While enough bytes remain...
        // ... see Frame Parse Logic below ...

        // 5. Shift unprocessed bytes to the front of the accumulator.
    }

    pub fn get_ptr(&self) -> *const f32 {
        self.output_pcm.as_ptr()
    }

    pub fn get_len(&self) -> usize {
        self.output_pcm.len()
    }
}
```

### 2.3 Variable Frame Parse Logic with Rollback
Inside the decode loop:
1. `let start_pos = reader.position();`
2. Read the 14-bit sync code. If it's not `0x3FFE`, advance 1 byte and continue.
3. Tentatively parse the variable length UTF-8 frame number.
4. Read the 16-bit block size.
5. **Sufficiency Check:** Calculate `required_bytes = current_pos_offset + (block_size * channels * (bps / 8)) + 2`.
6. If `accumulator.len() - start_pos < required_bytes`, the frame is incomplete. `reader.set_position(start_pos); break;` to wait for the next chunk.
7. Otherwise, proceed to read the verbatim subframes.
8. **Sign Extension & Normalization:**
   ```rust
   // For each channel's raw value:
   let sample_i32 = if bps == 24 {
       (raw_value as i32) << 8 >> 8
   } else { // 16-bit
       (raw_value as i32) << 16 >> 16
   };
   let float_val = sample_i32 as f32 / (1 << (bps - 1)) as f32;
   self.output_pcm.push(float_val);
   ```

## Validation
Write a standalone HTML file `test.html` that loads `flac_decoder.js`, instantiates the decoder, passes a static 24-bit `.flac` array, and extracts the `Float32Array` view using:
```javascript
const pcmView = new Float32Array(wasm.memory.buffer, decoder.get_ptr(), decoder.get_len());
```
Verify the array is populated with valid float data (values strictly between -1.0 and 1.0). Do the same with a 16-bit downsampled FLAC file to verify the dynamic bps extraction logic holds.
