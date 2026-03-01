# Local Development

This guide outlines the step-by-step process for running the full streaming system locally using Docker Compose.

## Prerequisites

1.  A local development environment (Linux, macOS, or WSL2) with Docker and Docker Compose installed.
2.  An analog audio source connected to the Behringer UMC404HD USB interface.
3.  The `UMC404HD` must be connected to the host machine and recognized by the host's ALSA system.

## Step-by-Step Guide

### 1. Clone the Repository

Clone the project to your local machine:

```bash
git clone <repository_url> radio-stream
cd radio-stream
```

### 2. Configure Environment

Ensure the `.env` file is present in the `radio-server/` directory. It should contain default values for local development:

```env
MINIO_USER=minioadmin
MINIO_PASS=minioadmin123
R2_BUCKET=radio-stream
```

### 3. Start the System

Navigate to the directory containing the `docker-compose.yml` file and start the services:

```bash
cd radio-server
docker compose up --build
```

Docker Compose will build the Rust server and Deno client images, start MinIO, initialize the bucket, and then launch the radio and client services.

### 4. Verify MinIO Console

Open a browser and navigate to the MinIO Console at `http://localhost:9001`.

*   Log in using the credentials from `.env` (`minioadmin` / `minioadmin123`).
*   Verify that the `radio-stream` bucket exists. After approximately 10 seconds of audio capture, a `live/manifest.json` will appear, and segment files will populate under the `live/hq/` and `live/lq/` prefixes. If no files appear after 30 seconds, check the `radio` container logs:
    ```bash
    docker compose logs radio
    ```
    Note: The browser fetches the manifest directly from `http://localhost:9000/radio-stream/live/manifest.json`, not from `http://localhost:3000`.

### 5. Open the Operator Monitor

Navigate to `http://localhost:8080`.

*   You should see the dark-themed operator monitor.
*   The "Live" badge should be green.
*   The Dual VU Meters should be animating in real-time if audio is flowing into the UMC404HD.
*   The "R2 Uploads" timeline should populate with segment pills as they are uploaded to MinIO.
*   Click the "Play" button under "Local Audio Monitor" to hear the live normalized stream directly from the server's RAM.

### 6. Open the Listener Interface

Navigate to `http://localhost:3000`.

*   You should see the light-themed public listener interface.
*   The live indicator dot should be pulsing green.
*   Click the large blue play button. You should hear the stream (delayed by the buffering latency, typically ~20-30 seconds behind the live source).
*   The waveform visualizer should animate.

### 7. Check the Local Archive

Verify that the uncompressed, raw archive is being written to disk. The `radio` service mounts a volume back to the host machine.

```bash
# From the radio-server directory
ls -la ./recordings
```

You should see a continuously growing FLAC file with a timestamped filename.