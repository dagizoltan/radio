# WASM Decoder Crate

The `decoder/flac/` directory contains the minimal FLAC Rust crate compiled to WebAssembly using `wasm-pack --target web`. It uses `wasm-bindgen` to expose a JavaScript API for decoding FLAC streams.

## Critical Constraint

**CRITICAL CONSTRAINT:** The decoder only needs to handle frames produced by our encoder. Do not implement the full FLAC specification.

Specifically, it only handles:
*   Verbatim subframes.
*   Block size code `0b0111` (16-bit literal).
*   Sample rate codes.
*   24-bit stereo (HQ) and 16-bit stereo (LQ).
*   No LPC, no Rice coding, no other subframe types.

## FlacDecoder Struct

The `FlacDecoder` struct maintains state across chunk pushes:

*   A byte accumulator buffer (`Vec<u8>`).
*   Parsed stream parameters (sample rate, channels, bps).
*   A `header_parsed` boolean flag.

The exact same WASM decoder class is used for both the HQ stream and the downsampled LQ stream. The decoder dynamically adjusts its parsing logic based on the parameters it reads from the incoming `STREAMINFO` block and the frame headers.

**Per-segment lifecycle:** Do not create a new `FlacDecoder` instance per segment to avoid main-thread allocation jank. Expose a `decoder.reset()` method that empties the accumulator buffer and sets `header_parsed` to `false`. Since each segment begins with a full FLAC stream header (`fLaC` + `STREAMINFO`), calling `reset()` ensures the next chunk correctly parses the header rather than misinterpreting it as frame data.

## push() Method API

The core JavaScript API is `push(bytes: &[u8]) -> Vec<f32>`.

1.  **Accumulation:** Incoming bytes are appended to the internal buffer.
2.  **Stream Header Parsing:** If `header_parsed` is false, it attempts to read the `STREAMINFO` block.
    *   It looks for the `fLaC` marker.
    *   It extracts sample rate (20 bits), channels (3 bits + 1), and bits per sample (5 bits + 1) using specific bit offsets from the packed metadata block.
    *   If successful, `header_parsed = true`.
3.  **Frame Decode Loop:** Once the header is parsed, it enters a loop attempting to decode as many full frames as possible from the buffer.
4.  **Consumption:** Processed bytes are removed from the front of the internal buffer. Unprocessed bytes (a partial frame at the end of a chunk) remain in the buffer for the next `push()` call. **Performance Critical:** Do not use `Vec::drain(..)` or `Vec::remove(0)` to remove processed bytes in a loop, as this causes `O(N^2)` memory shifting inside the WASM linear memory and can cause playback stutters on lower-end devices. Instead, track a read offset integer during the decode loop, and only use `Vec::drain(..offset)` or `slice::copy_within` once per `push()` call to move the unprocessed remainder to the front.
5.  **Return (Zero-Copy Optimization):** Instead of allocating a new `Vec<f32>` and copying it across the WASM/JS boundary, the decoder should maintain an internal output buffer in WASM linear memory. The `push()` method returns a pointer (and length) to this buffer. The JavaScript side constructs a `Float32Array` *view* directly over the WASM memory buffer, avoiding a massive garbage-collection-inducing copy operation on every chunk.

Important: The Float32Array view into WASM memory is valid for reading only. Do NOT transfer its underlying .buffer to the AudioWorklet via postMessage — doing so detaches WASM linear memory and crashes the decoder. Instead, use the buffer pool pattern described in player-component.md: create a view over the WASM output, copy into a pooled Float32Array, transfer the pooled buffer.

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
    *   Read `block_size` number of samples at `bps` bits each (e.g., 24 bits or 16 bits depending on the stream).
    *   Sign-extend the two's complement value to an i32:
        For 24-bit: `let sample: i32 = (raw_value as i32) << 8 >> 8;`
        For 16-bit: `let sample: i32 = (raw_value as i32) << 16 >> 16;`
        This arithmetic right shift propagates the sign bit correctly. Failure to sign-extend causes negative samples to appear as large positive values, producing severe distortion in the normalized f32 output.
4.  **Byte Alignment:** Align the reader to a byte boundary after subframes.
5.  **CRC-16:** Read the trailing CRC-16.
6.  **Sufficiency Check:** The frame size is only fully known after parsing the variable-length frame header (the UTF-8 frame number makes header length variable). The correct approach is a tentative parse with rollback:
    1. Record start_offset = current read position.
    2. Tentatively parse the frame header (sync, flags, UTF-8 frame number, literal block size, literal sample rate, CRC-8).
    3. Calculate required_bytes = bytes_consumed_by_header + (block_size * channels * bps / 8) + 2 (CRC-16).
    4. If buffer.len() - start_offset < required_bytes: reset read position to start_offset; break the decode loop.
    5. Otherwise: proceed with subframe decode and CRC-16 verification.
    This prevents partially decoded frames from corrupting the decoder state.

## BitReader Helper

The crate implements a `BitReader` struct that takes a byte slice and allows reading arbitrary bit counts, maintaining the current byte index and bit position within that byte.

## Normalization

The extracted 24-bit integer samples (`i32` internally) must be normalized to `f32` floats in the range `[-1.0, 1.0]` before returning them to JavaScript (as required by the Web Audio API).

**Formula:** `sample_f32 = sample_i32 as f32 / (1 << (bps - 1)) as f32`
(e.g., divide by `8388608.0` for 24-bit audio).