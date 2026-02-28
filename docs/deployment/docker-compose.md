# Docker Compose Orchestration

The system uses `radio-server/docker-compose.yml` to orchestrate all four necessary components for local development.

## Topology

The topology consists of four distinct services:

1.  **`minio`**: The local S3-compatible object storage server.
2.  **`minio-setup`**: A temporary container to configure the `minio` instance.
3.  **`radio`**: The Rust server application capturing and uploading audio.
4.  **`client`**: The Deno frontend serving the web listener interface.

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
*   **Lifecycle**: Exits immediately upon completion.

### 3. `radio`

*   **Build**: Built from `radio-server/Dockerfile`.
    *   Two-stage build: `rust:1.77-slim` for compilation, `debian:bookworm-slim` for runtime.
    *   Installs `pkg-config`, `libasound2-dev` (build time), and `libasound2` (runtime).
*   **Dependencies**: `depends_on: minio-setup` with the `condition: service_completed_successfully` flag.
*   **Ports**: Exposes port `8080` (Monitor UI).
*   **Device Passthrough**: Maps `/dev/snd` from the host to `/dev/snd` in the container.
*   **Volumes**: Mounts `./recordings` as a volume to persist recordings on the host machine.
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
    *   *Build-time Requirement:* The Dockerfile **must** install the Rust toolchain and `wasm-pack` to compile the `decoder` crate (`wasm-pack build --target web`) before the Deno server is started, ensuring the `.js` and `.wasm` files are available in `static/`.
    *   Pre-caches dependencies with `deno cache main.tsx`.
*   **Dependencies**: `depends_on: minio-setup` with the `condition: service_completed_successfully` flag.
*   **Ports**: Exposes port `3000` (Listener Interface).
*   **Environment Variables**:
    *   `R2_PUBLIC_URL`: `http://localhost:9000/${R2_BUCKET}`. The Hono server injects this URL into the frontend so the browser can fetch audio chunks directly from the local MinIO instance (simulating the Cloudflare R2 edge).
    *   `PORT`: `3000`

## Volumes

*   `minio-data`: A persistent named volume for the S3-compatible backend.

## Environment Files

The orchestration relies on two configuration files:

*   **`.env`**: Used by default for local development. Contains values like `MINIO_USER`, `MINIO_PASS`, and `R2_BUCKET`.
*   **`.env.prod`**: Used specifically for deploying the `radio-server` in production mode, pointing directly at Cloudflare R2 instead of the local MinIO instance. See [Production Deployment](production.md).