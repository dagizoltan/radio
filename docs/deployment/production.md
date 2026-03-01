# Production Deployment

Deploying the system for production involves reconfiguring the `radio-server` to target Cloudflare R2 and deploying the `radio-client` independently to Deno Deploy.

## Prerequisites

1.  A Cloudflare account with R2 enabled.
2.  A Deno Deploy account.
3.  **Strict NTP Synchronization:** The ThinkPad hardware clock must be perfectly synchronized to UTC. Install and run an NTP daemon (like `chrony` or `systemd-timesyncd`). This is critical because the rolling window logic, segment accumulation timing, and R2 metadata timestamps rely on a monotonically stable, non-drifting system clock to prevent the manifest edge from going out-of-sync with the client fetch requests over long uptimes.

### NTP Synchronisation (Required)

AWS Signature V4 request signing embeds a timestamp. R2 rejects any request whose timestamp deviates from server time by more than **5 minutes**, returning a silent `403 Forbidden` with no retry hint. Since these failures can be mistaken for permission or credential errors, the ThinkPad system clock must be kept tightly synchronised.

**Verify sync is active:**
```bash
# For systemd-timesyncd (default on Ubuntu):
timedatectl show-timesync --all
# Healthy output: NTPSynchronized=yes, SystemNTPServers populated

# For chrony:
chronyc tracking
# Healthy output: System time offset < 0.01 seconds, RMS offset < 0.1 seconds
```

**Acceptable offset threshold:** < 1 second at all times. If `timedatectl` shows `NTPSynchronized=no` or chrony shows offset > 10 seconds, investigate NTP connectivity before deploying. Configure the ThinkPad to use a nearby NTP pool (`pool.ntp.org` or a regional equivalent) in `/etc/systemd/timesyncd.conf` or `/etc/chrony.conf`.

**Clock Skew Mitigation:** Even with NTP, host clock drift can occur. The custom AWS Signature V4 implementation in the S3 Uploader task should proactively mitigate this by calculating a clock skew offset. It periodically fetches the `Date` header from an R2 response and adjusts the local `RequestDateTime` and `x-amz-date` accordingly, preventing spurious 403 errors when the system clock drifts slightly out of sync.

**Alarm:** If the S3 uploader begins receiving `403` responses after a period of success, check NTP sync status before investigating credentials.

> **Diagnosing 403 errors:** Check the R2 response XML body before assuming a credential problem. `RequestTimeTooSkewed` is a clock issue; `InvalidAccessKeyId` or `SignatureDoesNotMatch` indicate a credential or implementation bug. The uploader logs the error code at `ERROR` level. Check NTP sync (`timedatectl show-timesync`) before rotating credentials.

## Step 1: Cloudflare R2 Setup

1.  Log in to the Cloudflare dashboard.
2.  Navigate to **R2**.
3.  Create a new bucket (e.g., `my-radio-stream`).
4.  Navigate to **Settings** for the bucket.
5.  Enable **Public Access** (either via an R2.dev subdomain or by binding a custom domain). Note this URL as the `R2_PUBLIC_URL` for the client.
6.  **Configure CORS Policy:** In the bucket **Settings** page, navigate to **CORS Policy** and add the following rule, replacing the origin with your actual Deno Deploy URL:

```json
[
  {
    "AllowedOrigins": ["https://your-project.deno.dev"],
    "AllowedMethods": ["GET"],
    "AllowedHeaders": ["*"],
    "ExposeHeaders": ["ETag"],
    "MaxAgeSeconds": 3600
  }
]
```

**Why this is required:** The browser `<radio-player>` Web Component fetches both `manifest.json` and audio segments directly from R2 (cross-origin). Without this policy, all segment and manifest fetches are blocked by the browser's CORS enforcement. The `ExposeHeaders: ["ETag"]` entry is essential for the `If-None-Match` manifest polling optimisation to function correctly.

**Note:** This is configured via the Cloudflare dashboard UI or the Cloudflare API — it cannot be set via the S3 API or `aws s3api put-bucket-cors`. The format above matches the Cloudflare R2 dashboard's CORS JSON input.
7.  Navigate to **R2 API Tokens** and create a new token.
    *   Permissions: **Object Read & Write**.
    *   Specific Bucket: Select your new bucket.
8.  Copy the **Access Key ID**, **Secret Access Key**, and the **S3 API URL** (the endpoint URL).

## Step 2: Configure Server for Production

Create or update the `.env.prod` file in the `radio-server/` directory with the R2 credentials.

```env
R2_ENDPOINT=https://<ACCOUNT_ID>.r2.cloudflarestorage.com
R2_BUCKET=my-radio-stream
R2_ACCESS_KEY=<YOUR_ACCESS_KEY>
R2_SECRET_KEY=<YOUR_SECRET_KEY>
```

## Step 3: Run the Server in Production Mode

On the ThinkPad connected to the UMC404HD, start the Docker Compose setup, explicitly pointing it to the production environment file and specifying only the `radio` service.

```bash
cd radio-server
docker compose --env-file .env.prod up --build radio -d
```

This bypasses the local `minio` and `client` services. The server will now encode and upload segments directly to Cloudflare R2.

## Step 4: Deploy the Client to Deno Deploy

1.  Link your GitHub repository to Deno Deploy or use the `deployctl` CLI.
2.  Set the entrypoint file to `radio-client/main.tsx`.
3.  Configure the environment variables in the Deno Deploy dashboard:
    *   `R2_PUBLIC_URL`: The public URL configured in Step 1 (e.g., `https://pub-xxxxxx.r2.dev/my-radio-stream`).

Once deployed, users can visit the Deno Deploy URL to listen to the live stream.

## Production Performance and Limits

### Bandwidth Estimation

The ThinkPad has a real-world upload bandwidth of approximately **10.68 Mbps**.

- **HQ Segment Size:** 10s × 48000 Hz × 3 bytes × 2 channels = **2,880,000 bytes (~2.88 MB)**
- **LQ Segment Size:** Opus VBR at 128 kbps target × 10s = **~100–220 KB** (average ~160 KB). For worst-case upload bandwidth planning, use the upper bound of **~220 KB** per segment. See the CDN Edge Caching section of [Design Decisions](../architecture/decisions.md) for rationale.
- **Required Upload Speed (both streams, worst case):** `((2.88 + 0.22) × 8) / 10 ≈ 2.48 Mbps` continuous. At average LQ size (160 KB): `((2.88 + 0.16) × 8) / 10 ≈ 2.43 Mbps`.
- **Headroom:** The 10.68 Mbps connection provides comfortable headroom.

### Storage Estimation

The system maintains a rolling window of exactly 3 segments per quality stream on R2 at any given time.

- **HQ segments:** 3 × ~2.88 MB = **~8.64 MB**
- **LQ segments:** 3 × ~160 KB (average, VBR range 100–220 KB) = **~0.48 MB average**
- **Manifest:** negligible (~200 bytes)
- **Total steady-state: ~9.12 MB**

This is a predictable, bounded footprint ensuring costs on Cloudflare R2 remain minimal. The previous estimate of ~4.5 MB was based on compressed FLAC for HQ; the correct figure uses verbatim FLAC at ~2.88 MB per segment.
## Secret Management

The `.env.prod` file contains `R2_ACCESS_KEY` and `R2_SECRET_KEY` in plaintext. These credentials grant write access to the R2 bucket. Observe the following:

- `.env.prod` **must** be listed in `.gitignore`. Verify with `git check-ignore -v .env.prod` before committing.
- On the ThinkPad, set file permissions to `600` (owner read-only): `chmod 600 radio-server/.env.prod`.
- Preferred production approach: store credentials in a root-owned `EnvironmentFile` (e.g., `/etc/radio-server/secrets.env`, mode `600`) and reference it from the systemd unit:
  ```ini
  [Service]
  EnvironmentFile=/etc/radio-server/secrets.env
  ```
  This keeps secrets outside the repository directory entirely.
- Rotate the R2 API token every 90 days. Cloudflare R2 API tokens can be revoked and reissued without changing the bucket; only the `R2_ACCESS_KEY` and `R2_SECRET_KEY` values in the environment need updating.
- Never log the secret key. The AWS Sig V4 implementation must not include `R2_SECRET_KEY` in any `tracing` span or log output, even at `TRACE` level.

## Graceful Shutdown Procedure

Always use the timeout-controlled stop command to allow the server to complete any in-flight segment upload and flush the current archive staging file:

```bash
docker compose stop --timeout 30
```

**What happens during shutdown:**
1. Docker sends `SIGTERM` to the `radio` container.
2. The Rust binary's shutdown handler catches the signal.
3. The Recorder Task flushes and closes the current staging FLAC file, then moves it to `./recordings/`.
4. The Converter Task completes the current in-progress segment (or discards a partial one, logging its index).
5. The Cloud Uploader Task completes the in-flight S3 `PUT` (with up to 3 retries), then writes a final `manifest.json` with `"live": false`.
6. The binary exits cleanly.

**If the server is killed with SIGKILL or crashes:** The staging file in `/staging/` (the tmpfs mount) is abandoned with a partial final frame. Because `/staging/` is an in-memory tmpfs, the file is lost entirely on container restart — it is not recoverable from disk. The archive rotation script should detect gaps in the timestamped filename sequence and log them.
