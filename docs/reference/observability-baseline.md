# Observability Baseline

This document defines healthy operating ranges and alarm thresholds for all telemetry metrics exposed by the system. Values outside the alarm range should trigger operator investigation.

## Server Metrics

| Metric | Source | Healthy Range | Alarm Threshold | Notes |
|---|---|---|---|---|
| Capture buffer overruns | Recorder Task | 0 / hour | > 0 | Any overrun means audio was lost from the archive. Caused by ALSA `EPIPE` (xrun) — the kernel filled the capture ring buffer before the Recorder Task read it. Investigate CPU scheduling spikes or increase the ALSA buffer size (currently 4 periods × 4096 frames). |
| ALSA period read latency | Recorder Task | < 2ms | > 10ms | Time from `AsyncFd::readable()` wakeup to `IOCTL_READI_FRAMES` completion. |
| Segment assembly time | Converter Task | 9.9–10.1s | < 9.5s or > 10.5s | Should be very close to 10s. Deviation indicates a sample rate mismatch between capture and encoder. |
| Segment upload exhaustion count | Uploader Task | 0 / hour | > 0 | A skipped segment means a gap in the live stream. Investigate R2 connectivity. |
| S3 PUT latency (HQ) | Uploader Task | < 3s | > 8s | Time for a single `PUT` of ~2.88 MB. If consistently > 8s, upload is slower than segment production. |
| S3 PUT latency (LQ) | Uploader Task | < 0.5s | > 3s | LQ Opus segments range 100–220 KB (VBR). Even at the upper bound, upload should complete well under 1 second on a 10 Mbps connection. |
| S3 PUT retry rate | Uploader Task | 0 retries / hour | > 3 retries / hour | Frequent retries indicate network instability or R2 availability issues. |
| Rolling window size | Uploader Task | exactly 3 | ≠ 3 | Should always be exactly 3 segments on R2. More = leak; fewer = startup or delete failure. |
| Manifest age (seconds since `updated_at`) | Uploader Task | < 15s | > 40s | Manifest should update every 10s. Staleness > 40s means the uploader has stalled. |

## Client Metrics (Reported to Analytics Endpoint)

| Metric | Healthy Range | Alarm Threshold | Notes |
|---|---|---|---|
| Segment fetch time (HQ) | 1–4s | > 8s | Time to fully download one 10s HQ segment. Auto-downgrade should trigger at > 8s. |
| Segment fetch time (LQ) | < 1s | > 4s | LQ Opus averages ~160 KB (VBR range 100–220 KB depending on content complexity). Even at the 220 KB upper bound, fetch should complete in under 1 second on any reasonable connection. |
| Worklet buffer depth | 1.0–3.0 segments | < 0.3 or > 4.5 | Measured as `samplesAvailable / (48000 * 2)` seconds. Too low → imminent underrun. Too high → latency growth. |
| Playback stall rate | 0 / hour | > 1 / hour | Number of times the worklet output silence due to underrun. Each stall is an audible glitch. |
| Latency behind live edge | 10–25s | > 45s | `(latest - currentIndex) * 10` seconds. Excessive latency indicates the client is not using jump-ahead logic correctly. |
| Quality auto-downgrade events | 0–1 / hour | > 3 / hour | Frequent auto-downgrades indicate sustained poor network conditions for the listener. |

## Prometheus Endpoint Format

The `radio-server` HTTP task exposes metrics at `GET /metrics` in Prometheus text format:

```
# HELP radio_capture_overruns_total Total ALSA buffer overruns since start
# TYPE radio_capture_overruns_total counter
radio_capture_overruns_total 0

# HELP radio_s3_put_latency_seconds Last S3 PUT latency
# TYPE radio_s3_put_latency_seconds gauge
radio_s3_put_latency_seconds{quality="hq"} 1.82
radio_s3_put_latency_seconds{quality="lq"} 0.08

# HELP radio_rolling_window_size Current number of segments on R2
# TYPE radio_rolling_window_size gauge
radio_rolling_window_size 3
```

The `/metrics` endpoint is also listed in [API Routes](api-routes.md). It is served by the local HTTP task on `127.0.0.1:8080` and is not exposed to the public internet. For remote monitoring, use SSH port forwarding: `ssh -L 9090:127.0.0.1:8080 operator@thinkpad` and configure Prometheus to scrape `localhost:9090/metrics`.
