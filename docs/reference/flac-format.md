# FLAC Format Subset

This document details the specific subset of the Free Lossless Audio Codec (FLAC) standard implemented by the [Encoder](../radio-server/encoder.md) and [WASM Decoder](../radio-client/wasm-decoder.md).

**Interop Note:** The FLAC files produced by this system are 100% valid, standard FLAC. They use "verbatim subframes" (uncompressed data), which is fully supported by all standard FLAC players.

## Stream Header

Every FLAC stream (and every standalone segment file) must begin with exactly 38 bytes:

1.  **Marker (4 bytes):** The ASCII string `fLaC` (`0x66`, `0x4c`, `0x61`, `0x43`).
2.  **STREAMINFO Block (34 bytes):**
    *   Block header: `0x00` (type 0: STREAMINFO, last-metadata-block flag unset) followed by 24-bit length `0x000022` (34 bytes).
    *   Min block size: `16` bits
    *   Max block size: `16` bits
    *   Min frame size: `24` bits (can be 0 if unknown)
    *   Max frame size: `24` bits (can be 0 if unknown)
    *   Sample rate: `20` bits (`48000`)
    *   Channels: `3` bits (`1` for 2 channels)
    *   Bits per sample: `5` bits (`23` for 24-bit)
    *   Total samples: `36` bits (`0` if streaming/unknown)
    *   MD5 signature: 16 bytes (all zeros if uncalculated).

## Frame Header

A FLAC frame begins with a variable-length header. Our implementation uses fixed codes for simplicity.

1.  **Sync Code (14 bits):** Always `0x3FFE`.
2.  **Reserved (1 bit):** Always `0`.
3.  **Blocking Strategy (1 bit):** Always `0` (fixed-blocksize).
4.  **Block Size Code (4 bits):** Always `0b0111` (indicates a 16-bit literal follows at the end of the header).
5.  **Sample Rate Code (4 bits):** Always `0b1100` (indicates a 24-bit literal follows at the end of the header).
6.  **Channel Assignment (4 bits):** Always `0b0001` (Left/Right stereo).
7.  **Sample Size Code (3 bits):** Always `0b110` (24 bits per sample).
8.  **Reserved (1 bit):** Always `0`.
9.  **Frame Number (Variable):** UTF-8 encoded integer.
10. **Literal Block Size (16 bits):** Present because of code `0b0111`. Value is `4096 - 1` (`4095`).
11. **Literal Sample Rate (24 bits):** Present because of code `0b1100`. Value is `48000`.
12. **CRC-8 (8 bits):** Computed over all bytes from the sync code up to (but not including) the CRC-8 byte itself. Polynomial `x^8 + x^2 + x^1 + x^0` (`0x07`).

## Verbatim Subframe

Following the frame header are two subframes (Left channel, then Right channel).

1.  **Subframe Header Byte (8 bits):**
    *   Zero bit: `0`
    *   Subframe type (6 bits): `0b000001` (Type 1: Verbatim). Note: The spec documentation describes this as type `000001` followed by 0 waste bits, making the full byte `0b00000010` (value 2).
    *   Wasted bits flag (1 bit): `0`.
2.  **Uncompressed Audio Data:** `4096` consecutive 24-bit samples for that channel, stored as big-endian (or written byte-aligned). Since bits-per-sample is 24 (3 bytes), this aligns perfectly to byte boundaries.

## CRC-16

At the very end of the frame (after both subframes), a 16-bit CRC is written.

*   **Scope:** Computed over the entire frame, starting from the first byte of the sync code (`0xFF`) through the last byte of the right channel's verbatim subframe data.
*   **Polynomial:** `x^16 + x^15 + x^2 + x^0` (`0x8005`).
## CRC Implementation Reference

Both the encoder and decoder implement two CRC algorithms. The following parameters fully specify each so that any compatible implementation (e.g., a Python verification script, a Go archive reader) can produce matching checksums.

### CRC-8 (Frame Header Integrity)

| Parameter | Value |
|---|---|
| Width | 8 bits |
| Polynomial | `0x07` (x⁸ + x² + x + 1) |
| Initial value | `0x00` |
| Input reflection | No |
| Output reflection | No |
| Final XOR | `0x00` |
| Check value (for "123456789") | `0xF4` |

Scope: all bytes from and including the sync code byte `0xFF` up to but not including the CRC-8 byte itself.

### CRC-16 (Frame Integrity)

| Parameter | Value |
|---|---|
| Width | 16 bits |
| Polynomial | `0x8005` (x¹⁶ + x¹⁵ + x² + 1) |
| Initial value | `0x0000` |
| Input reflection | No |
| Output reflection | No |
| Final XOR | `0x0000` |
| Check value (for "123456789") | `0xFEE8` |

Scope: all bytes from and including the sync code byte `0xFF` through the last byte of the right channel's verbatim subframe data.

### Lookup Table vs. Bitwise
Both CRCs should be implemented using a precomputed 256-entry lookup table for performance in the hot encoding path. The table is generated once at startup. For the WASM decoder, the table is computed at module initialisation time and stored as a static `[u8; 256]` (CRC-8) or `[u16; 256]` (CRC-16).

### Test Vectors

| Input bytes | CRC-8 | CRC-16 |
|---|---|---|
| `[]` (empty) | `0x00` | `0x0000` |
| `[0xFF, 0xF8]` (sync + flags) | to be computed against implementation | to be computed against implementation |

Implementers must verify their CRC routines against the FLAC reference decoder (`flac --test`) on a known-good output file from this encoder.

## UTF-8 Encoded Frame Numbers

FLAC frame headers encode the frame number as a UTF-8 style variable-length integer (not UTF-8 codepoints — the same bit-packing scheme is reused for integers). The decoder must parse this field correctly to locate the end of the frame header.

| Frame number range | Byte count | Bit pattern |
|---|---|---|
| 0 – 127 | 1 | `0xxxxxxx` |
| 128 – 2,047 | 2 | `110xxxxx 10xxxxxx` |
| 2,048 – 65,535 | 3 | `1110xxxx 10xxxxxx 10xxxxxx` |
| 65,536 – 1,048,575 | 4 | `11110xxx 10xxxxxx 10xxxxxx 10xxxxxx` |
| 1,048,576 – 34,359,738,367 | 5–6 | Extended patterns (see FLAC spec §11) |

**At 48000 Hz, 10s segments:** The frame number crosses the 1-byte boundary (127) after ~21 minutes of broadcast. It crosses the 2-byte boundary (2047) after ~5.7 hours. It crosses the 3-byte boundary (65,535) after ~7.6 days. It crosses the 4-byte boundary (1,048,575) after ~121 days. At the 8-digit segment index (100M segments), each segment contains `480,000 / 4096 ≈ 117` FLAC frames, meaning the frame counter wraps the 4-byte boundary during day 121 of continuous operation. The decoder must handle all byte widths up to 6.

**Decoder implementation:** After reading the sync code and flags, call a `read_utf8_int(reader) -> u64` helper. This helper reads the first byte, inspects the leading bits to determine total byte count, then reads the continuation bytes, masking off the leading `10` bits and assembling the value. The result is the frame number; its byte length is needed to correctly position the reader before the literal block size / sample rate fields.
