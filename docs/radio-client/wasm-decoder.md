# WASM Decoder Crate

The `decoder/` directory contains a minimal Rust crate compiled to WebAssembly using `wasm-pack --target web`. It uses `wasm-bindgen` to expose a JavaScript API for decoding FLAC streams.

## Critical Constraint

**CRITICAL CONSTRAINT:** The decoder only needs to handle frames produced by our encoder. Do not implement the full FLAC specification.

Specifically, it only handles:
*   Verbatim subframes.
*   Block size code `0b0111` (16-bit literal).
*   Sample rate code `0b1100` (24-bit literal).
*   24-bit stereo.
*   No LPC, no Rice coding, no other subframe types.

## OpusDecoder (LQ Stream)

The `decoder/opus/` crate exposes an `OpusDecoder` class for the LQ Opus-in-Ogg stream. It uses the `opus-rs` crate (safe Rust bindings to the reference libopus implementation) and the `ogg` crate for Ogg packet framing. Both compile to WASM via `wasm-pack --target web`.

### push() Method API

The API is identical to `FlacDecoder.push()`: `push(bytes: &[u8]) -> Vec<f32>`.

1.  **Ogg Sync:** Incoming bytes are fed to an Ogg `PacketReader`. Ogg provides its own sync and resync mechanism — if bytes arrive mid-page, the reader discards data until the next page capture pattern (`OggS`).
2.  **Opus Header Pages:** The first two Ogg pages are the Opus header and comment header. These are parsed to extract channel count and pre-skip samples. They produce no PCM output.
3.  **Decode Loop:** Subsequent Ogg pages contain Opus audio packets. Each packet is passed to `OpusDecoder::decode_float()`, producing interleaved `f32` PCM at 48000 Hz (Opus natively operates at 48 kHz).
4.  **Pre-skip:** The first `pre_skip` samples of decoded audio are discarded (per the Opus spec) to account for encoder lookahead delay.
5.  **Return:** Decoded `f32` samples are returned in the same format as `FlacDecoder`: interleaved stereo, range `[-1.0, 1.0]`, ready for the AudioWorklet ring buffer.

### Ogg Page Boundary Behaviour

Unlike FLAC frames, Opus packets are page-aligned by the Ogg container. If a chunk boundary falls mid-page, the `PacketReader` buffers the partial page and waits for the next `push()` call to complete it. No explicit frame-boundary detection is required — Ogg handles synchronisation.

### Segment Boundary Handling

Each 10-second `.opus` segment is a complete, self-contained Ogg Opus stream (begins with the Opus header pages). When the player advances to the next segment index, a new `OpusDecoder` instance is created (calling `decoder.free()` first to release WASM memory). This avoids any Ogg stream state carried across segment boundaries.


## FlacDecoder Struct

The `FlacDecoder` struct maintains state across chunk pushes:

*   A byte accumulator buffer (`Vec<u8>`).
*   Parsed stream parameters (sample rate, channels, bps).
*   A `header_parsed` boolean flag.

## push() Method API

The core JavaScript API is `push(bytes: &[u8]) -> Vec<f32>`.

1.  **Accumulation:** Incoming bytes are appended to the internal buffer.
2.  **Stream Header Parsing:** If `header_parsed` is false, it attempts to read the `STREAMINFO` block.
    *   It looks for the `fLaC` marker.
    *   It extracts sample rate (20 bits), channels (3 bits + 1), and bits per sample (5 bits + 1) using specific bit offsets from the packed metadata block.
    *   If successful, `header_parsed = true`.
3.  **Frame Decode Loop:** Once the header is parsed, it enters a loop attempting to decode as many full frames as possible from the buffer.
4.  **Consumption:** Processed bytes are removed from the front of the internal buffer. Unprocessed bytes (a partial frame at the end of a chunk) remain in the buffer for the next `push()` call.
5.  **Return (Zero-Copy Optimization):** Instead of allocating a new `Vec<f32>` and copying it across the WASM/JS boundary, the decoder should maintain an internal output buffer in WASM linear memory. The `push()` method returns a pointer (and length) to this buffer. The JavaScript side constructs a `Float32Array` *view* directly over the WASM memory buffer, avoiding a massive garbage-collection-inducing copy operation on every chunk.

## Frame Decode Process

For each frame in the loop:

1.  **Sync Detection:** Search for the `0x3FFE` sync code in the first 14 bits.
2.  **Header Parse:** Read the frame header:
    *   Block size code.
    *   Sample rate code.
    *   Channel count.
    *   BPS code.
    *   UTF-8 encoded frame number (variable length, 1-6 bytes).
    *   Actual block size (16-bit read).
    *   Actual sample rate (24-bit read).
    *   CRC-8 (must match).
3.  **Subframe Decode:** Decode two verbatim subframes (Left, then Right).
    *   Read subframe header byte.
    *   Read `block_size` number of samples at `bps` bits each (e.g., 24 bits).
    *   Sign-extend the 24-bit two's complement value to an `i32`.
4.  **Byte Alignment:** Align the reader to a byte boundary after subframes.
5.  **CRC-16:** Read the trailing CRC-16.
6.  **Sufficiency Check:** *Crucially, before committing to a frame decode, the decoder must check if enough bytes exist in the buffer to complete the entire frame (header + samples + CRC).* If not, it breaks the loop and waits for the next `push()`.

## BitReader Helper

The crate implements a `BitReader` struct that takes a byte slice and allows reading arbitrary bit counts, maintaining the current byte index and bit position within that byte.

## Normalization

The extracted 24-bit integer samples (`i32` internally) must be normalized to `f32` floats in the range `[-1.0, 1.0]` before returning them to JavaScript (as required by the Web Audio API).

**Formula:** `sample_f32 = sample_i32 as f32 / (1 << (bps - 1)) as f32`
(e.g., divide by `8388608.0` for 24-bit audio).