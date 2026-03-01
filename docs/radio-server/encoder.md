# Encoder Crate

The `crates/encoder` library provides a pure Rust FLAC encoder. It produces valid FLAC files using verbatim subframes, meaning the audio samples are uncompressed but wrapped in lossless FLAC framing.

## FLAC Stream Structure

A valid FLAC stream consists of:
1.  **Stream Header:** A 4-byte `fLaC` marker followed by a `STREAMINFO` metadata block.
2.  **Encoded Frames:** One or more FLAC frames containing the audio data.

The `FlacEncoder` struct manages the stream configuration and provides two methods: `stream_header()` and `encode_frame()`.

## FlacEncoder API

The `FlacEncoder` holds the configuration:
*   Sample Rate: 48000
*   Channels: 2
*   Bits per sample: 24
*   Block Size: 4096
*   Frame Counter: (internal state)

It exposes two methods:
*   `stream_header() -> Vec<u8>`: Returns the complete stream header (`fLaC` marker + `STREAMINFO` block). This is called once per encoder instance and cached.
*   `encode_frame(interleaved: &[i32]) -> &[u8]`: Takes interleaved samples and returns a slice pointing into an internal, pre-allocated output buffer containing exactly one FLAC frame.

**Contract:** `encode_frame` must perform zero allocations in the hot path. The returned slice is valid until the next call.

**Frame counter continuity:** The `FlacEncoder` instance used by the Converter Task is **reused across segments** — it is created once at Converter startup and lives for the process lifetime. The internal frame counter increments monotonically. Each 10-second segment assembled by prepending the cached stream header to accumulated frames will therefore contain frames with non-zero frame numbers. This is valid per the FLAC specification; frame numbers are used for seeking, not for standalone-file validity. The `STREAMINFO` block's `total_samples` field is set to `0` (streaming/unknown), which is also valid.

**Do not** create a new `FlacEncoder` per segment. Doing so would reset the frame counter to 0 on every segment, producing duplicate frame numbers across the stream if a player were to concatenate segments — a minor compliance issue with no practical impact, but unnecessary.

## Verbatim Subframe Layout

Each FLAC frame encodes one block of audio (4096 frames). It contains:

1.  **Sync Code:** 14 bits set to `0x3FFE`.
2.  **Frame Header:**
    *   Block size code (`0b0111`, indicating a 16-bit literal size follows).
    *   Sample rate code (`0b1100`, indicating a 24-bit literal rate follows).
    *   Channel assignment (`0b0001` for stereo left/right).
    *   Bits-per-sample code (`0b110` for 24 bits).
    *   UTF-8 encoded frame number (variable length, 1-6 bytes).
    *   Actual block size (16-bit, `4096 - 1`).
    *   Actual sample rate (24-bit, `48000`).
    *   **CRC-8** of the frame header bytes so far.
3.  **Subframes:** One subframe per channel (left, then right).
    *   Subframe type byte (`0b00000010` indicating verbatim, 0 waste bits).
    *   Raw 24-bit samples for that channel.
4.  **CRC-16:** Of the entire frame, from the sync code to the end of the last subframe.

## BitWriter

The crate implements a `BitWriter` helper struct that writes arbitrary bit counts to an internal byte buffer, handling byte alignment automatically. It is used to write the packed fields in the `STREAMINFO` block and the frame header.

## Critical Constraints

**CRITICAL CONSTRAINT:** The encoder only needs to handle producing verifiable verbatim subframes. Do not implement the full FLAC specification for encoding (no LPC, no Rice coding, no other subframe types).