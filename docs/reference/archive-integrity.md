# Archive Integrity Verification

The HQ Recorder produces bit-perfect 24-bit/48 kHz FLAC files in `./recordings/`. This document describes how to verify their integrity.

## Verification Tool

Use the official FLAC CLI:
```bash
flac --test recordings/*.flac
```
A clean file produces: `recordings/recording-1234.flac: ok`
A corrupt file produces: `recordings/recording-1234.flac: ERROR while decoding data` with a non-zero exit code.

## Integration with Archive Rotation

The cold-storage rotation cron job must run integrity verification before uploading to the archive bucket:
```bash
#!/bin/bash
# Run as part of daily archive rotation
FAILED=()
for f in ./recordings/*.flac; do
    flac --test --silent "$f" || FAILED+=("$f")
done

if [ ${#FAILED[@]} -gt 0 ]; then
    echo "INTEGRITY FAILURE: ${FAILED[*]}" >&2
    # Alert operator; do NOT upload corrupt files; do NOT delete locally
    exit_status=1
fi

# Upload verified files to cold storage S3 bucket
# ... aws s3 cp commands ...

# Only delete locally after confirmed upload
```

## Partial Final Frames

If the server was killed with SIGKILL or crashed, the last FLAC file may contain an incomplete final frame. `flac --test` will report this as an error. These files are partially recoverable: use `ffmpeg -i recording.flac -c:a flac recovered.flac` to recover all complete frames and discard the trailing partial frame. Log the frame count loss and keep both the original and recovered versions.

## Expected File Sizes

| Duration | Expected size (verbatim 24-bit/48 kHz stereo) |
|---|---|
| 1 hour | ~1.04 GB |
| 8 hours | ~8.3 GB |
| 24 hours | ~24.9 GB |

Significant deviation from these ranges (>10%) may indicate a capture problem (clocked at wrong sample rate, wrong bit depth, or mono instead of stereo).
