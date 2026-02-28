# Normalizer Crate

The `crates/normalizer` library provides a two-stage audio normalizer operating on interleaved `i32` samples (representing 24-bit audio) in place.

## Stage 1: LUFS Gain Rider

The first stage targets a consistent -14 LUFS level by adjusting the overall gain dynamically based on short-term loudness measurements.

1.  **Measurement:** It measures the RMS (Root Mean Square) level of the audio in 100ms blocks.
2.  **Windowing:** It maintains a circular window of 30 RMS blocks, representing 3 seconds of audio history.
3.  **Gain Calculation:** After each block, it computes the total window RMS in dB. It calculates the necessary gain adjustment to reach the target of -14 LUFS.
4.  **Clamping:** The target gain is clamped strictly between -12 dB and +6 dB.
5.  **Smoothing:** The actual applied gain is smoothed to prevent sudden volume jumps. It uses separate time constants:
    *   **Attack:** ~500ms time constant. This reacts relatively quickly when the signal gets louder (preventing prolonged clipping).
    *   **Release:** ~2000ms time constant. This reacts slowly when the signal gets quieter (preventing "pumping" artifacts during momentary lulls).
6.  **Application:** The current smoothed gain is applied multiplicatively to every sample in the block.
7.  **Warm-up State:** To prevent a massive +6dB gain jump when the process starts (because the 3-second RMS history window is initially empty/silence), the normalizer bypasses gain application (stays at 0dB adjustment) for the first 3 seconds until the window is fully populated with actual audio data.

This stage preserves the original dynamics (transients) of the performance while gently riding the overall volume level to ensure consistency across different records or mixer settings.

## Stage 2: True Peak Limiter

The second stage is a fast-acting envelope follower that acts as a safety net against digital clipping (exceeding 0 dBFS), which sounds harsh and distorted.

1.  **Envelope Follower:** It maintains a per-sample envelope that tracks the highest recent peaks.
2.  **Attack/Release:** Instant attack (reacts immediately to a peak) and a 50ms release.
3.  **Threshold:** The threshold is set to -1 dBFS (linear value `~0.891`).
4.  **Scaling:** If the envelope exceeds the threshold, the sample is scaled down proportionally to ensure it never exceeds -1 dBFS.

This stage only acts when a transient peak is too loud, compressing the peak transparently without affecting the overall body of the audio below the threshold.

## API and Implementation Details

The `Normalizer` struct exposes a `process` method:

*   `process(&mut [i32]) -> f32`: Takes a mutable slice of interleaved `i32` samples. Applies both normalization stages in-place. Returns the currently applied smoothed gain in dB (used for the monitor UI).

**Internal Processing:** The normalizer temporarily converts the `i32` samples to `f32` for all calculations and gain application, ensuring precision. After processing, it converts the `f32` samples back to `i32`, applying clamping to the 24-bit range (`-8388608` to `8388607`) to prevent integer overflow or clipping beyond the 24-bit boundary.

## Critical Constraints

**CRITICAL CONSTRAINT:** The normalizer never touches the recorded audio. The pipeline task passes a mutable copy of the buffer to the normalizer *after* the raw samples have been encoded and broadcast to the recorder task.