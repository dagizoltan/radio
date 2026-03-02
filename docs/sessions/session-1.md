# Prompt for Session 1: Core Capture and Encoding (The Foundation)

**Goal:** Implement the foundational pure-Rust crates for ALSA capture and FLAC encoding. Prove we can grab analog sound and losslessly serialize it.

**Context & Requirements:**
You are to build the first layer of the Lossless Vinyl Radio Streaming System: the `capture` and `encoder` crates within the `radio-server` workspace.

**1. Capture Crate (`crates/capture`):**
- **Device Discovery (`src/discovery.rs`):** Parse `/proc/asound/cards`. Read `CAPTURE_DEVICE_NAME`. If it matches a card name, extract the card number `{N}` and return the path `/dev/snd/pcmC{N}D0c`.
- **ALSA Structs (`src/alsa_sys.rs`):** Define the exact `#[repr(C)]` memory layouts matching Linux `<sound/asound.h>` for `SndrPcmHwParams`, `SndInterval`, and `SndrPcmXferi`. Hardcode the specific hex ioctls (`SNDRV_PCM_IOCTL_HW_PARAMS`, etc).
- **Configuration (`src/device.rs`):** Open the device `O_RDWR | O_NONBLOCK`. Call `ioctl` to request: `FORMAT_S24_LE` (value 6), `ACCESS_RW_INTERLEAVED` (value 3), `48000` Hz, `2` channels, `4096` frames per period, `4` periods buffer.
- **Validation:** Crucially, read back the `hw_params` struct after the ioctl. Verify the device didn't fallback to a different rate or format. Panic if it did.
- **Async Loop (`src/capture.rs`):** Wrap the `RawFd` in `tokio::io::unix::AsyncFd`. Await `.readable()`. Call `IOCTL_READI_FRAMES`.
- **Sign-Extension:** Iterate over the raw `u32` words from the ALSA buffer. Extract the 24-bit audio using: `let sample: i32 = (raw_word as i32) << 8 >> 8;`. Collect these into a `Vec<i32>`.
- **EPIPE (XRUN) Recovery:** If `IOCTL_READI_FRAMES` returns `EPIPE` (xrun):
  1. Log a warning and increment the overrun counter.
  2. Synthesize a zero-padded buffer of exactly `4096 * 2` (8192) `0i32` values to represent the lost period of silence.
  3. Call `IOCTL_PREPARE` to reset the hardware.
  4. Return the silence buffer to ensure the archive encoder maintains structural continuity.

**2. Encoder Crate (`crates/encoder`):**
- **BitWriter (`src/bitwriter.rs`):** Implement a struct that accepts a bit count and a value (`write_bits(val: u64, bits: u8)`), accumulating bits into a `Vec<u8>` without byte-aligning between calls.
- **STREAMINFO (`src/flac.rs`):** The `stream_header()` method must produce:
  - `fLaC` marker.
  - `0x80` block header byte (indicating last metadata block) + 24-bit length `0x000022`.
  - Bit-packed fields: `16`-bit min block, `16`-bit max block, `24`-bit min frame (0), `24`-bit max frame (0), `20`-bit sample rate (48000), `3`-bit channels (1 for stereo), `5`-bit bps (23 for 24-bit), `36`-bit total samples (0), and `128`-bit MD5 (0).
- **Frame Writing:** `encode_frame(interleaved: &[i32], frame_number: u64)`:
  - Write sync code `0x3FFE`.
  - Write fixed codes for block size (`0b0111`), rate (`0b1100`), channels (`0b0001`), and bps (`0b110`).
  - Write UTF-8 encoded `frame_number`.
  - Write literal block size (`4095`) and rate (`48000`).
  - Calculate and write CRC-8 of the header.
  - Write subframe headers (`0b00000010` for verbatim) and the raw 24-bit samples byte-aligned.
  - Calculate and write CRC-16 of the entire frame.
- **CRCs (`src/crc.rs`):** Implement table-driven CRC-8 (`0x07`) and CRC-16 (`0x8005`).

**Validation:**
Include a simple test/binary that captures 10 seconds of audio from the physical interface and outputs a `test.flac` file. The file must play perfectly in VLC and pass FLAC validation.
