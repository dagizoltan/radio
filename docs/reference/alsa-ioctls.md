# ALSA ioctls

The [Capture Crate](../radio-server/capture.md) interacts with the Linux kernel's ALSA subsystem using raw `ioctl`s, completely bypassing `libasound`.

This document lists the constants and struct layouts derived from `<sound/asound.h>` required to configure the Behringer UMC404HD.

## Device Path

*   **Pattern:** `/dev/snd/pcmC{N}D0c`
*   `C{N}` is the card number (e.g., `C1`), discovered dynamically by parsing `/proc/asound/cards`.
*   `D0` is device 0.
*   `c` indicates capture (vs `p` for playback).
*   **Open Flags:** `O_RDWR | O_NONBLOCK`

## ioctl Constants

These hex values are platform-specific (Linux x86_64/ARM64) and hardcoded into the crate.

*   `IOCTL_HW_PARAMS`: `0xc2604111`
*   `IOCTL_PREPARE`: `0x4140`
*   `IOCTL_READI_FRAMES`: `0x80184151`

## #[repr(C)] Structs

These Rust structs must exactly match the C memory layout expected by the kernel.

### SndInterval

Used to specify ranges for sample rates, channels, and buffer sizes.

```rust
#[repr(C)]
struct SndInterval {
    min: u32,
    max: u32,
    flags: u32, // bit 0: openmin, bit 1: openmax, bit 2: integer, bit 3: empty
}
```

### SndrPcmHwParams

The core configuration payload sent via `IOCTL_HW_PARAMS`.

```rust
#[repr(C)]
struct SndrPcmHwParams {
    flags: u32,
    masks: [u32; 16], // Bitmasks for format, access, etc.
    mres: [u32; 20],
    intervals: [SndInterval; 12], // rate, channels, period_size, etc.
    ires: [u32; 36],
    rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    fifo_size: u64,
    reserved: [u8; 64],
}
```

### SndrPcmXferi

The payload used for reading interleaved frames via `IOCTL_READI_FRAMES`.

```rust
#[repr(C)]
struct SndrPcmXferi {
    result: i64,      // Number of frames actually read
    buf: *mut i32,    // Pointer to the user-space buffer (for 24-bit audio, typically packed into 32-bit words by ALSA)
    frames: u64,      // Number of frames requested
}
```

## Configuration Sequence

1.  **Open:** Open the device file descriptor with `O_RDWR | O_NONBLOCK`. This is required so `AsyncFd` can manage readiness without blocking the thread.
2.  **`IOCTL_HW_PARAMS`:** Construct the `SndrPcmHwParams` struct specifying `FORMAT_S24_LE` (24-bit little-endian), `ACCESS_RW_INTERLEAVED` (LRLRLR...), 48000 Hz, 2 channels, 4096 period size, and 4 periods per buffer. Execute the ioctl.
3.  **`IOCTL_PREPARE`:** Prepare the hardware for capture.
4.  **`AsyncFd` Loop:**
    *   Await `readable()`.
    *   Construct `SndrPcmXferi` pointing to a `&mut [i32]` buffer sized for 4096 frames (8192 samples).
    *   Execute `IOCTL_READI_FRAMES`.
    *   If `result > 0`, process audio. If `EWOULDBLOCK`, loop.