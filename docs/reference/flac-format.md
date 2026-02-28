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
    *   Sample rate: `20` bits (`44100`)
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
11. **Literal Sample Rate (24 bits):** Present because of code `0b1100`. Value is `44100`.
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