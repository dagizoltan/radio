# AWS Signature V4 & Cloud Uploading

The `radio-server` uploads segments to Cloudflare R2 (or MinIO in development).

## Deprecation of Custom SigV4 Implementation

Historically, this project maintained a custom, from-scratch implementation of AWS Signature Version 4 to reduce dependencies. However, manually managing clock skew mitigation, connection pooling, and exponential backoff retries proved too fragile for long-term production stability.

**The system now relies on robust, standard crates for S3 interaction.**

## Recommended S3 Integration

The Cloud Uploader Task must use a high-level S3 client rather than raw `reqwest` calls. The recommended crates are:

1.  **`object_store`**: A highly robust, production-ready abstraction for object storage. It supports S3 natively, handles SigV4 seamlessly, manages its own connection pooling, and implements intelligent exponential backoff and retry logic out-of-the-box.
2.  **`aws-sdk-s3`**: The official AWS Rust SDK. While heavier than `object_store`, it is the industry standard and guarantees perfect SigV4 compliance.

## Transitioning from Raw Reqwest

If migrating from the legacy `reqwest` implementation, the Uploader Task should be refactored to initialize an `object_store::aws::AmazonS3` instance at startup.

*   **Clock Skew:** Rely on the underlying crate's handling of `403 RequestTimeTooSkewed` rather than implementing manual Date header parsing.
*   **Retries:** Use the crate's built-in retry mechanisms instead of wrapping PUTs in a manual loop.
*   **Payloads:** Pass the assembled segment bytes as a `Bytes` stream to the `put` method.

## Critical Constraints

**CRITICAL CONSTRAINT:** All S3 operations use path-style URLs: `{endpoint}/{bucket}/{key}`. This is necessary because the server must work identically with local MinIO (which typically lacks DNS support for virtual hosts) and Cloudflare R2. Virtual-hosted style URLs (`{bucket}.{endpoint}/{key}`) are not used. Configure the chosen S3 crate (e.g., `object_store` or `aws-sdk-s3`) to force path-style access.