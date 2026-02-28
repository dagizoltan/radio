# Environment Variables

This document lists all environment variables used across the system, indicating which service uses them and where they are read.

| Variable | Service | Default | Description | Read Location |
| :--- | :--- | :--- | :--- | :--- |
| `R2_ENDPOINT` | `radio-server` | (None) | Base URL for the S3 API (MinIO or R2). | Read by `reqwest` client initialization in the S3 Uploader task. |
| `R2_BUCKET` | `radio-server`, `client` (via Compose) | `radio-stream` | The name of the S3 bucket to upload to. | Read by the S3 Uploader task for path construction; read by docker-compose to construct `R2_PUBLIC_URL`. |
| `R2_ACCESS_KEY` | `radio-server` | (None) | The S3 access key ID for signing requests. | Read by the [AWS Signature V4](../radio-server/aws-sig-v4.md) implementation. |
| `R2_SECRET_KEY` | `radio-server` | (None) | The S3 secret access key for signing requests. | Read by the [AWS Signature V4](../radio-server/aws-sig-v4.md) implementation. |
| `R2_PUBLIC_URL` | `radio-client` | (None) | The public-facing base URL for fetching the manifest and segments. | Injected into HTML as `data-r2-url`; the Deno server never proxies using it. |
| `PORT` | `radio-client` | `3000` | The HTTP port the Hono server binds to. | Read by `Deno.env.get("PORT")` in `main.tsx`. |
| `MINIO_USER` | `docker-compose` (`minio`) | (None) | The root username for the local MinIO instance. | Read by Docker Compose to set `MINIO_ROOT_USER` and pass to `radio-server` as `R2_ACCESS_KEY`. |
| `MINIO_PASS` | `docker-compose` (`minio`) | (None) | The root password for the local MinIO instance. | Read by Docker Compose to set `MINIO_ROOT_PASSWORD` and pass to `radio-server` as `R2_SECRET_KEY`. |
| `RUST_LOG` | `radio-server` | `info` | The log level for the Rust binary. | Read by `tracing_subscriber::EnvFilter::from_default_env()` on startup. |