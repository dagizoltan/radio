# AWS Signature V4

The `radio-server` uploads segments to Cloudflare R2 (or MinIO in development) using a custom, from-scratch implementation of AWS Signature Version 4. It does not use the official AWS SDK or external S3 crates.

## Crates Used

The implementation relies solely on cryptographic primitives:
*   `hmac` (HMAC-SHA256)
*   `sha2` (SHA256)
*   `hex` (Hexadecimal encoding)

## The Signing Process

Uploading to S3 requires signing the HTTP request headers and payload.

### 1. Signing Key Derivation

A distinct signing key is derived for each date and service.

1.  Start with `HMAC-SHA256("AWS4" + secret_key, date_string)`. The `date_string` is formatted as `YYYYMMDD`. Let this be `k_date`.
2.  Compute `HMAC-SHA256(k_date, region_string)`. The region string is always `"auto"`. Let this be `k_region`.
3.  Compute `HMAC-SHA256(k_region, service_string)`. The service string is always `"s3"`. Let this be `k_service`.
4.  Compute `HMAC-SHA256(k_service, "aws4_request")`. This is the final binary `signing_key`.

### 2. Canonical Request Construction

The HTTP request is normalized into a "Canonical Request" string.

**Clock Skew Mitigation:** R2 rejects requests where the `x-amz-date` and `RequestDateTime` deviate from the server's time by more than 5 minutes. To mitigate host clock drift without relying exclusively on NTP, the S3 Uploader Task should periodically fetch the `Date` header from an R2 response (e.g., via a `HEAD` request or by interpreting the headers of a failed `PUT`) to calculate a "clock skew" offset relative to the local system clock. This calculated offset must be added to the current system time when generating the `x-amz-date` and `RequestDateTime` fields.

```text
HTTPRequestMethod
CanonicalURI
CanonicalQueryString
CanonicalHeaders
SignedHeaders
HashedPayload
```

*   **CanonicalURI:** The path part of the URL.
*   **CanonicalQueryString:** Empty for standard PUTs.
*   **CanonicalHeaders:** Lowercase header names followed by values, sorted alphabetically by name. Required headers: `content-type`, `host`, `x-amz-content-sha256`, `x-amz-date`.
*   **SignedHeaders:** A semicolon-separated list of the header names used in CanonicalHeaders (e.g., `content-type;host;x-amz-content-sha256;x-amz-date`).
*   **HashedPayload:** The SHA256 hash of the request body (the FLAC segment bytes), hex-encoded.

**Critical implementation note:** The value of the `x-amz-content-sha256` request header must be set to the **same hex-encoded SHA256 string** as `HashedPayload` in the canonical request. Compute the hash once, store it in a variable, and use it in both places. If they differ — even by case — the signature will not match and R2 will return `403 SignatureDoesNotMatch`.

### 3. String to Sign

The string to sign incorporates the canonical request hash and the signing context.

```text
Algorithm
RequestDateTime
CredentialScope
HashedCanonicalRequest
```

*   **Algorithm:** `"AWS4-HMAC-SHA256"`
*   **RequestDateTime:** `YYYYMMDDThhmmssZ`
*   **CredentialScope:** `{date_string}/{region}/{service}/aws4_request`
*   **HashedCanonicalRequest:** The SHA256 hash of the Canonical Request string, hex-encoded.

### 4. Calculating the Signature

The final signature is calculated using the derived signing key.

`Signature = HexEncode(HMAC-SHA256(signing_key, StringToSign))`

### 5. Authorization Header

The final `Authorization` header is constructed and attached to the raw HTTP `reqwest` client.

`AWS4-HMAC-SHA256 Credential={access_key}/{CredentialScope}, SignedHeaders={SignedHeaders}, Signature={Signature}`

## Critical Constraints

**CRITICAL CONSTRAINT:** All S3 operations use path-style URLs: `{endpoint}/{bucket}/{key}`. This is necessary because the server must work identically with local MinIO (which typically lacks DNS support for virtual hosts) and Cloudflare R2. Virtual-hosted style URLs (`{bucket}.{endpoint}/{key}`) are not used.