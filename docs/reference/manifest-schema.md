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
| `latest` | `number` (safe integer) | 8-digit-compatible integer. The index of the most recently completed and uploaded segment. Always in the range `0` – `99,999,999`, which is safely representable as a JavaScript `number` (well below `Number.MAX_SAFE_INTEGER` of 2⁵³ − 1). No `BigInt` handling is required. Wraps at 100,000,000 via modular arithmetic on the server. |
| `segment_s` | `u32` | Duration of each segment in seconds. Always `10` in this implementation. Used by the client for latency calculation. |
| `updated_at` | `u64` | Unix timestamp in milliseconds of the last successful segment upload. Used by the client to detect stale manifests (if `updated_at` is more than `segment_s * 3` seconds in the past, the stream may be down). |
| `qualities` | `string[]` | Available quality levels. Always `["hq", "lq"]`. Used by the player to validate the quality selector options. |

## Client Usage

- **Live status badge:** Set from `live`.
- **Latency display:** `(latest - currentIndex) * segment_s` seconds.
- **Jump-ahead:** If `currentIndex < latest - 3`, snap to `latest - 1`.
- **Rollover detection:** If `latest < currentIndex && currentIndex - latest > 3`, treat as rollover; snap to `latest`.
- **Stale stream detection:** If `Date.now() - updated_at > segment_s * 3 * 1000`, show "Stream may be offline" warning even if `live: true`.
- **Quality URL construction:** `${r2Url}/live/${quality}/segment-${String(latest).padStart(8,'0')}.${quality==='hq'?'flac':'opus'}`
