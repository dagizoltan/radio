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
# Run as part of daily archive rotation.
# Exits non-zero if any file fails integrity check.
# Never uploads or deletes corrupt files.

set -euo pipefail

RECORDING_DIR="./recordings"
FAILED=()

echo "Running FLAC integrity checks..."
for f in "${RECORDING_DIR}"/*.flac; do
    [ -e "$f" ] || { echo "No FLAC files found in ${RECORDING_DIR}"; exit 0; }
    if flac --test --silent "$f"; then
        echo "  OK: $f"
    else
        echo "  FAIL: $f" >&2
        FAILED+=("$f")
    fi
done

if [ ${#FAILED[@]} -gt 0 ]; then
    echo "" >&2
    echo "INTEGRITY FAILURE — the following files are corrupt:" >&2
    printf '  %s\n' "${FAILED[@]}" >&2
    echo "" >&2
    echo "Action: Do NOT upload or delete these files. Investigate manually." >&2
    echo "Tip: Use 'ffmpeg -i <file> -c:a flac recovered.flac' to recover complete frames." >&2
    exit 1
fi

echo "All integrity checks passed. Proceeding with upload..."

# Upload verified files to cold storage S3 bucket.
# Replace the following with your actual upload command:
# aws s3 cp "${RECORDING_DIR}/" s3://your-cold-storage-bucket/recordings/ --recursive --exclude "*.tmp"

echo "Upload complete. Removing local copies..."

# Only delete locally after confirmed upload exits 0.
# rm "${RECORDING_DIR}"/*.flac
```

> **`set -euo pipefail` explained:** `set -e` exits immediately if any command fails. `set -u` treats unset variables as errors. `set -o pipefail` ensures a pipeline's exit code is the first non-zero exit code. Together, these prevent silent partial failures in the rotation script.

## Partial Final Frames

If the server was killed with SIGKILL or crashed, the last FLAC file may contain an incomplete final frame. `flac --test` will report this as an error. These files are partially recoverable: use `ffmpeg -i recording.flac -c:a flac recovered.flac` to recover all complete frames and discard the trailing partial frame. Log the frame count loss and keep both the original and recovered versions.

## Expected File Sizes

| Duration | Expected size (verbatim 24-bit/48 kHz stereo) |
|---|---|
| 1 hour | ~1.04 GB |
| 8 hours | ~8.3 GB |
| 24 hours | ~24.9 GB |

Significant deviation from these ranges (>10%) may indicate a capture problem (clocked at wrong sample rate, wrong bit depth, or mono instead of stereo).
