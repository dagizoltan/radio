# Capture Crate

The `crates/capture` library is responsible for reading digital audio directly from the Linux kernel ALSA interface. It bypasses all C libraries (`libasound2`) in favor of direct kernel communication.

## ALSA Device Discovery

The capture device is a Behringer UMC404HD. The crate locates the correct ALSA PCM device file dynamically at runtime.

1.  It parses `/proc/asound/cards`.
2.  It matches the string `"UMC404"`.
3.  It extracts the card number (e.g., `C1`).
4.  It constructs the device path: `/dev/snd/pcmC{N}D0c`, where `{N}` is the card number.

## Raw ioctl Interface

The crate configures the ALSA device using raw kernel `ioctl`s wrapped by `rustix`.

### Device Configuration

The device is opened with `O_RDWR | O_NONBLOCK`.

*   **Format:** `FORMAT_S24_LE` (or `FORMAT_S32_LE` depending on hardware `hw_params`)
*   **Access Mode:** `ACCESS_RW_INTERLEAVED`
*   **Sample Rate:** 48000 Hz
*   **Channels:** 2
*   **Period Size:** 4096 frames
*   **Buffer Size:** 4 periods

*(Note: The `S24_LE` ALSA format uses 32-bit words (4 bytes per sample), where the audio data occupies the lower 24 bits and the top 8 bits are zero-padded. The hardware may natively expose `S32_LE` or `S24_3LE` (tightly packed 3 bytes). The implementer must log and check the supported formats and ensure the 24 bits are correctly extracted and packed tightly (3 bytes per sample) before verbatim FLAC encoding.)*

### #[repr(C)] Structs

The crate defines Rust structs with `#[repr(C)]` that exactly match the memory layout of the kernel's `<sound/asound.h>` structures:

*   `SndrPcmHwParams`: Contains flag masks and `SndInterval` arrays.
*   `SndInterval`: Min, max, flags.
*   `SndrPcmXferi`: Result `i64`, buf pointer, frames `u64`.

The `ioctl` constants (e.g., `IOCTL_HW_PARAMS`, `IOCTL_PREPARE`, `IOCTL_READI_FRAMES`) are hardcoded hex values derived directly from the Linux kernel headers. See the [ALSA ioctls Reference](../reference/alsa-ioctls.md) for the exact values.

## AsyncFd Wakeup Model

The capture loop uses Tokio's `AsyncFd` for zero-polling, kernel-driven wakeups.

1.  The raw file descriptor is wrapped in `tokio::io::unix::AsyncFd`.
2.  The `read_period` method awaits `async_fd.readable()`. This puts the Tokio task to sleep.
3.  When the kernel signals the ALSA file descriptor is readable (a period of 4096 frames is ready), Tokio wakes the task.
4.  The task calls `try_io` with the `IOCTL_READI_FRAMES` ioctl to read the period into an `&mut [i32]` buffer.
5.  If the read is successful, it returns the number of samples read. If it returns `EWOULDBLOCK`, the task loops back to await readability.

## Critical Constraints

**CRITICAL CONSTRAINT:** No C bindings in capture. The capture crate must not link against `libasound` or any C audio library. All ALSA interaction is via raw kernel ioctls through `rustix`.

**CRITICAL CONSTRAINT:** ALSA device discovery is dynamic. The card number for the UMC404HD is found at runtime by parsing `/proc/asound/cards`. Do not hardcode card numbers.

**CRITICAL CONSTRAINT:** Tokio AsyncFd for capture, not threads. The audio capture must use `AsyncFd` so the Tokio runtime controls wakeup. Do not spawn a dedicated OS thread for capture or use `spawn_blocking` with a polling loop.