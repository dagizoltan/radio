# Docker Compose Orchestration

The system uses `radio-server/docker-compose.yml` to orchestrate all four necessary components for local development.

## Topology

The topology consists of four distinct services:

1.  **`minio`**: The local S3-compatible object storage server.
2.  **`minio-setup`**: A temporary container to configure the `minio` instance.
3.  **`radio`**: The Rust server application capturing and uploading audio.
4.  **`client`**: The Deno frontend serving the web listener interface. All audio segment and manifest fetches are performed browser-side directly against `http://localhost:9000/${R2_BUCKET}`. The Deno client serves only the HTML shell and static assets.

## Service Details

### 1. `minio`

*   **Image**: `quay.io/minio/minio:latest`
*   **Ports**: Maps host port `9000` (S3 API) to container port `9000`. Maps host port `9001` (Console) to container port `9001`.
*   **Volumes**: Mounts a named volume, `minio-data`, to persist data across container restarts.
*   **Environment Variables**:
    *   `MINIO_ROOT_USER`: Pulled from `.env` (`MINIO_USER`).
    *   `MINIO_ROOT_PASSWORD`: Pulled from `.env` (`MINIO_PASS`).
*   **Healthcheck**: A crucial configuration that continuously polls the MinIO live health endpoint:
    *   `test: ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]`
    *   `interval: 5s`
    *   `timeout: 5s`
    *   `retries: 5`

### 2. `minio-setup`

*   **Image**: `quay.io/minio/mc:latest`
*   **Dependencies**: `depends_on: minio` with the `condition: service_healthy` flag. This ensures setup only begins after the storage backend is fully initialized.
*   **Execution**: Runs a shell one-liner to initialize the bucket and access policies:
    *   Sets an alias for the local `minio` service.
    *   Creates the target bucket (e.g., `radio-stream`) using `--ignore-existing`.
    *   Sets the anonymous download policy to allow public reads directly from the client browser.


After setting the anonymous download policy, configure a CORS policy to allow direct browser `GET` requests:

```bash
mc anonymous set-json /tmp/cors.json minio/radio-stream
```

Where `/tmp/cors.json` contains:
```json
{
  "CORSRules": [{
    "AllowedOrigins": ["http://localhost:3000"],
    "AllowedMethods": ["GET"],
    "AllowedHeaders": ["*"],
    "ExposeHeaders": ["ETag"],
    "MaxAgeSeconds": 3600
  }]
}
```

`ExposeHeaders: ["ETag"]` is required so the browser's `If-None-Match` manifest polling optimisation (which now hits MinIO directly) functions correctly.

*   **Lifecycle**: Exits immediately upon completion.

### 3. `radio`

*   **Build**: Built from `radio-server/Dockerfile`.
    *   Two-stage build: `rust:1.77-slim` for compilation, `debian:bookworm-slim` for runtime.
    *   Installs `pkg-config`, `libasound2-dev` (build time), and `libasound2` (runtime).
    *   **CI Constraint:** After building, validate the binary does not dynamically link `libasound`: `ldd target/release/radio-server | grep asound` must return no output. If `alsa-sys` appears in `Cargo.lock`, fail the build.
*   **Dependencies**: `depends_on: minio-setup` with the `condition: service_completed_successfully` flag.
*   **Ports**: Exposes port `8080` (Monitor UI).
*   **Device Passthrough**: Maps `/dev/snd` from the host to `/dev/snd` in the container.
*   **Volumes**: Mounts `./recordings` as a volume to persist recordings on the host machine.
*   **tmpfs**: Mounts a tmpfs at `/staging` for fast RAM-backed staging file writes: `tmpfs: ["/staging:size=64m,mode=1777"]`.
*   **Audio Group GID Fix**: The entrypoint is a custom shell script that resolves permissions for the ALSA device.
    1.  Reads the GID of `/dev/snd/controlC0` from the host.
    2.  Adjusts the container's `audio` group to match the host GID.
    3.  Adds the runtime user to the `audio` group.
    4.  `exec`s the Rust binary.
*   **Environment Variables**:
    *   `R2_ENDPOINT`: `http://minio:9000`
    *   `R2_BUCKET`: Pulled from `.env`
    *   `R2_ACCESS_KEY`: Pulled from `.env` (`MINIO_USER`)
    *   `R2_SECRET_KEY`: Pulled from `.env` (`MINIO_PASS`)
    *   `RUST_LOG`: `info`

### 4. `client`

*   **Build**: Built from `radio-client/Dockerfile`.
    *   Uses `denoland/deno:2.0.0` or a multi-stage Rust+Deno image.
    *   *Build-time Requirement:* The Dockerfile **must** install the Rust toolchain and `wasm-pack` to compile **two** WASM crates: `wasm-pack build --target web` for `decoder/flac/` (producing `flac_decoder.js` + `flac_decoder_bg.wasm`) and for `decoder/opus/` (producing `opus_decoder.js` + `opus_decoder_bg.wasm`). Both sets of outputs must be present in `static/` before the Deno server starts.
    *   Pre-caches dependencies with `deno cache main.tsx`.
*   **Dependencies**: `depends_on: minio-setup` with the `condition: service_completed_successfully` flag.
*   **Ports**: Exposes port `3000` (Listener Interface).
*   **Environment Variables**:
    *   `R2_PUBLIC_URL`: `http://localhost:9000/${R2_BUCKET}`. Injected into the rendered HTML as a `data-r2-url` attribute on `<radio-player>`. The Deno server does not proxy traffic using this URL.
    *   `PORT`: `3000`

## Volumes

*   `minio-data`: A persistent named volume for the S3-compatible backend.

The `/staging` tmpfs is sized at 64 MB. At ~1.5 GB/hour, a single 60-minute staging file reaches ~1.04 GB — well above 64 MB. The Recorder Task must therefore rotate the staging file to `./recordings/` more frequently than once per hour. The recommended rotation interval is **every 10 minutes** (~240 MB per staging file), which fits comfortably within the 64 MB limit.

## Environment Files

The orchestration relies on two configuration files:

*   **`.env`**: Used by default for local development. Contains values like `MINIO_USER`, `MINIO_PASS`, and `R2_BUCKET`.
*   **`.env.prod`**: Used specifically for deploying the `radio-server` in production mode, pointing directly at Cloudflare R2 instead of the local MinIO instance. See [Production Deployment](production.md).
## Auto-Start on Boot (Systemd)

To ensure the radio server restarts automatically after a host reboot (e.g., following a kernel update), create a systemd service unit on the ThinkPad:

```ini
# /etc/systemd/system/radio-server.service
[Unit]
Description=Lossless Vinyl Radio Server
Requires=docker.service
After=docker.service network-online.target
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=/home/operator/radio-stream/radio-server
ExecStart=/usr/bin/docker compose up -d --build
ExecStop=/usr/bin/docker compose stop --timeout 30
TimeoutStartSec=120
TimeoutStopSec=45

[Install]
WantedBy=multi-user.target
```

Enable with:
```bash
sudo systemctl enable radio-server.service
sudo systemctl start radio-server.service
```

**Shutdown behaviour:** `docker compose stop --timeout 30` sends `SIGTERM` to each container and waits up to 30 seconds. The `radio` container's Rust binary must catch `SIGTERM` via `tokio::signal::unix::signal(SignalKind::terminate())` and initiate graceful shutdown: flush the current staging file, close the ALSA device, complete any in-flight S3 upload, and update the manifest to `"live": false` before exiting. See [Graceful Shutdown](../radio-server/server.md) for the implementation contract.
