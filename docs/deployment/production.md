# Production Deployment

Deploying the system for production involves reconfiguring the `radio-server` to target Cloudflare R2 and deploying the `radio-client` independently to Deno Deploy.

## Prerequisites

1.  A Cloudflare account with R2 enabled.
2.  A Deno Deploy account.

## Step 1: Cloudflare R2 Setup

1.  Log in to the Cloudflare dashboard.
2.  Navigate to **R2**.
3.  Create a new bucket (e.g., `my-radio-stream`).
4.  Navigate to **Settings** for the bucket.
5.  Enable **Public Access** (either via an R2.dev subdomain or by binding a custom domain). Note this URL as the `R2_PUBLIC_URL` for the client.
6.  Navigate to **R2 API Tokens** and create a new token.
    *   Permissions: **Object Read & Write**.
    *   Specific Bucket: Select your new bucket.
7.  Copy the **Access Key ID**, **Secret Access Key**, and the **S3 API URL** (the endpoint URL).

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

*   **Segment Size:** The target size is equivalent to 10 seconds of raw PCM audio (approx. 1,764,000 bytes or ~1.76 MB).
*   **Required Upload Speed:** The server must upload 1.76 MB every 10 seconds, which translates to a required continuous upload speed of approximately **1.41 Mbps** (`(1.76 * 8) / 10`).
*   **Headroom:** The 10.68 Mbps connection provides massive headroom, ensuring segments upload rapidly and the stream remains stable even with network fluctuations.

### Storage Estimation

The system maintains a rolling window of exactly 3 segments on R2 at any given time.

*   **Steady-State Size:** 3 segments * ~1.5 MB (FLAC compressed) + 1 manifest file ≈ **4.5 MB total storage**.
*   This predictable, bounded footprint ensures costs on Cloudflare R2 remain minimal.