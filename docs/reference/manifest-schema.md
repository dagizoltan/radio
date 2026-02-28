# Manifest Schema

`live/manifest.json` is the single coordination point between the server and all browser clients. It is uploaded to S3 after every segment with `Cache-Control: no-store, max-age=0` to prevent CDN caching.

## Schema

```json
{
  "live": true,
  "latest": 42,
  "segment_s": 10,
  "updated_at": 1718000000000,
  "qualities": ["hq", "lq"]
}
```

| Field | Type | Description |
|---|---|---|
| `live` | `boolean` | `true` if the server is actively uploading segments. Set to `false` during startup sweep and on graceful shutdown. |
| `latest` | `number` (safe integer, max 99,999,999) | The index of the most recently completed and uploaded segment. Always in the range `0`–`99,999,999`, well below `Number.MAX_SAFE_INTEGER`. Wraps at 100,000,000 via modular arithmetic on the server. No BigInt handling required. |
| `segment_s` | `number` (integer, always `10`) | Duration of each segment in seconds. Used by the client for latency calculation and stale-manifest detection. |
| `updated_at` | `number` (Unix ms timestamp) | Unix timestamp in milliseconds of the last successful segment upload. Safely representable as a JavaScript `number` until the year 2255. Used by the client to detect stale manifests: if `Date.now() - updated_at > segment_s * 3 * 1000`, show a "Stream may be offline" warning. |
| `qualities` | `string[]` | Available quality levels. Always `["hq", "lq"]`. Used by the player to validate the quality selector options. |

> **All numeric fields** are within JavaScript's safe integer range (`Number.MAX_SAFE_INTEGER` = 2⁵³ − 1 ≈ 9 quadrillion). No `BigInt` handling is required anywhere in the client implementation.

## Client Usage

- **Live status badge:** Set from `live`.
- **Latency display:** `(latest - currentIndex) * segment_s` seconds.
- **Jump-ahead:** If `currentIndex < latest - 3`, snap to `latest - 1`.
- **Rollover detection:** If `latest < currentIndex && currentIndex - latest > 3`, treat as rollover; snap to `latest`.
- **Stale stream detection:** If `Date.now() - updated_at > segment_s * 3 * 1000`, show "Stream may be offline" warning even if `live: true`.
- **Quality URL construction:** `${r2Url}/live/${quality}/segment-${String(latest).padStart(8,'0')}.${quality==='hq'?'flac':'opus'}`
