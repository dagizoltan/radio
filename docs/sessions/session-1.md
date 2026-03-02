# Prompt for Session 1: Core Capture and Encoding (The Foundation)

**Goal:** Implement the foundational pure-Rust crates for ALSA capture and FLAC encoding. Prove we can grab analog sound and losslessly serialize it.

**Context & Requirements:**
You are to build the first layer of the Lossless Vinyl Radio Streaming System: the `capture` and `encoder` crates within the `radio-server` workspace.

**1. Capture Crate (`crates/capture`):**
- **Dynamic Device Discovery:** Parse `/proc/asound/cards` to find the correct ALSA PCM device file dynamically based on the `CAPTURE_DEVICE_NAME` environment variable (e.g., `"UMC404"`).
- **Raw `rustix` IOCTLs:** Configure the ALSA device using strictly raw kernel `ioctl`s via `rustix`. DO NOT use `libasound` or C bindings. The configuration must request: `FORMAT_S24_LE` (or fallback to device native and map correctly), `ACCESS_RW_INTERLEAVED`, 48000 Hz, 2 channels, 4096 frames per period, and 4 periods buffer.
- **Hardware Parameter Validation:** Read back the `hw_params` after setting them to ensure the kernel didn't silently fall back to an unsupported configuration.
- **AsyncFd Wakeups:** Wrap the raw file descriptor in `tokio::io::unix::AsyncFd`. Wait for kernel-driven wakeups via `.readable()` instead of blocking polls.
- **Sign-Extension:** Apply the critical sign-extension algorithm `let sample: i32 = (raw_word as i32) << 8 >> 8;` for `S24_LE` words to avoid severe distortion on negative samples.
- **EPIPE (XRUN) Recovery:** When `IOCTL_READI_FRAMES` returns `EPIPE`, write exactly 4096 frames of pure silence (zero-padded) to the archive encoder to prevent structural timeline discontinuities. Then call `IOCTL_PREPARE` to reset.

**2. Encoder Crate (`crates/encoder`):**
- **Verbatim FLAC Encoder:** Write a pure-Rust FLAC encoder that ONLY outputs verbatim subframes (no LPC, no Rice coding).
- **STREAMINFO Header:** The `stream_header()` method must produce a valid 34-byte `STREAMINFO` block. Crucially, the block header byte MUST be `0x80` (flagging it as the last metadata block).
- **Bit-packing:** All STREAMINFO fields must be sequentially bit-packed with a `BitWriter` (no byte alignment padding between fields).
- **Frame Writing:** `encode_frame(interleaved: &[i32])` must accept raw samples and output exactly one valid FLAC frame (sync code `0x3FFE`, variable UTF-8 frame number, 16-bit block literal, 24-bit rate literal, verbatim subframes, CRC-16).
- **CRCs:** Implement the CRC-8 (polynomial `0x07`) and CRC-16 (polynomial `0x8005`) using precomputed 256-entry lookup tables.

**Validation:**
Include a simple test/binary that captures 10 seconds of audio from the physical interface and outputs a `test.flac` file. The file must play perfectly in VLC and pass FLAC validation.
