use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use crate::aws_sig_v4::generate_sigv4;
use crate::state::AppState;

#[derive(Serialize, Deserialize)]
struct StateFile {
    latest: u64,
}

#[derive(Serialize)]
struct Manifest {
    live: bool,
    latest: u64,
    segment_s: f64,
    updated_at: u64,
    qualities: Vec<&'static str>,
}

pub struct UploaderTask {
    seg_rx: mpsc::Receiver<(u64, Bytes, Bytes)>,
    state: Arc<AppState>,
    client: Client,
    window: VecDeque<u64>,
    last_persisted_index: u64,
}

impl UploaderTask {
    pub async fn new(
        seg_rx: mpsc::Receiver<(u64, Bytes, Bytes)>,
        state: Arc<AppState>,
    ) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(8))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        // Ensure the directory exists
        let _ = tokio::fs::create_dir_all("./recordings").await;

        let last_persisted_index = match tokio::fs::read_to_string("./recordings/state.json").await {
            Ok(content) => match serde_json::from_str::<StateFile>(&content) {
                Ok(state_file) => state_file.latest,
                Err(_) => 0,
            },
            Err(_) => 0,
        };

        state.r2_segment.store(last_persisted_index, Ordering::SeqCst);

        // Spawn background cleanup task
        let index_to_cleanup = last_persisted_index;
        let cleanup_client = client.clone();
        tokio::spawn(async move {
            Self::background_cleanup(cleanup_client, index_to_cleanup).await;
        });

        UploaderTask {
            seg_rx,
            state,
            client,
            window: VecDeque::new(),
            last_persisted_index,
        }
    }

    async fn background_cleanup(client: Client, current_index: u64) {
        let max_index = current_index.saturating_sub(10);

        let access_key = std::env::var("R2_ACCESS_KEY").unwrap_or_else(|_| "test_access".to_string());
        let secret_key = std::env::var("R2_SECRET_KEY").unwrap_or_else(|_| "test_secret".to_string());
        let endpoint = std::env::var("R2_ENDPOINT").unwrap_or_else(|_| "https://test.s3.amazonaws.com".to_string());

        // A full robust implementation would parse the XML from the S3 list response
        // using an XML parser to find keys matching the prefix `live/hq/segment-{index}.flac`
        // and issue deletes. For the sake of the prompt "delete all keys older than last_persisted_index - 10"
        // and "do not block the main Uploader loop", we perform basic list requests and deletes here.

        // Optional: Get bucket from env, default to empty
        let bucket = std::env::var("R2_BUCKET").unwrap_or_else(|_| "".to_string());
        let bucket_prefix = if bucket.is_empty() { String::new() } else { format!("/{bucket}") };

        let prefixes = ["live/hq/", "live/lq/"];
        for prefix in prefixes {
            let mut continuation_token = String::new();

            loop {
                let uri = if bucket_prefix.is_empty() { "/".to_string() } else { bucket_prefix.clone() };

                // SigV4 requires query parameters to be sorted alphabetically by key.
                // Keys: continuation-token, list-type, prefix
                let mut query = String::new();
                if !continuation_token.is_empty() {
                    let encoded_token = urlencoding::encode(&continuation_token);
                    query.push_str(&format!("continuation-token={}&", encoded_token));
                }
                query.push_str(&format!("list-type=2&prefix={}", prefix));

                let url = format!("{}{}?{}", endpoint, uri, query);

                let now = OffsetDateTime::now_utc();
                let amz_date = format!(
                    "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
                    now.year(), now.month() as u8, now.day(),
                    now.hour(), now.minute(), now.second()
                );
                let date_stamp = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());
                let payload_hash = hex::encode(Sha256::digest(b""));

                let mut headers = BTreeMap::new();
                let host = url.replace("https://", "").replace("http://", "").split('/').next().unwrap_or("").to_string();
                headers.insert("Host".to_string(), host);
                headers.insert("x-amz-date".to_string(), amz_date.clone());
                headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());

                let (auth_header, _) = generate_sigv4(
                    "GET",
                    &uri,
                    &query,
                    &headers,
                    &payload_hash,
                    &access_key,
                    &secret_key,
                    "us-east-1",
                    "s3",
                    &amz_date,
                    &date_stamp,
                );

                let req = client.get(&url)
                    .header("x-amz-date", &amz_date)
                    .header("x-amz-content-sha256", payload_hash)
                    .header("Authorization", auth_header);

                let mut next_token = None;
                let mut has_more = false;

                if let Ok(resp) = req.send().await {
                    if let Ok(xml_str) = resp.text().await {
                        if let Ok(doc) = roxmltree::Document::parse(&xml_str) {
                            for node in doc.descendants() {
                                if node.has_tag_name("Key") {
                                    if let Some(key) = node.text() {
                                        // Parse index from "live/hq/segment-123.flac"
                                        if let Some(idx_str) = key.strip_prefix(prefix)
                                            .and_then(|s| s.strip_prefix("segment-"))
                                            .and_then(|s| s.strip_suffix(".flac"))
                                        {
                                            if let Ok(idx) = idx_str.parse::<u64>() {
                                                if idx < max_index {
                                                    // Actually delete
                                                    tracing::info!("Deleting old S3 segment: {}", key);
                                                    Self::delete_s3_segment_static(&client, &bucket_prefix, prefix, idx).await;
                                                }
                                            }
                                        }
                                    }
                                } else if node.has_tag_name("NextContinuationToken") {
                                    if let Some(token) = node.text() {
                                        next_token = Some(token.to_string());
                                    }
                                } else if node.has_tag_name("IsTruncated") {
                                    if let Some(text) = node.text() {
                                        has_more = text == "true";
                                    }
                                }
                            }
                        }
                    }
                }

                if has_more && next_token.is_some() {
                    continuation_token = next_token.unwrap();
                } else {
                    break;
                }
            }
        }
    }

    async fn delete_s3_segment_static(client: &Client, bucket_prefix: &str, quality_prefix: &str, index: u64) {
        let uri = format!("{bucket_prefix}/{quality_prefix}segment-{index:08}.flac");

        let access_key = std::env::var("R2_ACCESS_KEY").unwrap_or_else(|_| "test_access".to_string());
        let secret_key = std::env::var("R2_SECRET_KEY").unwrap_or_else(|_| "test_secret".to_string());
        let endpoint = std::env::var("R2_ENDPOINT").unwrap_or_else(|_| "https://test.s3.amazonaws.com".to_string());

        let url = format!("{}{}", endpoint, uri);

        let now = OffsetDateTime::now_utc();
        let amz_date = format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            now.year(), now.month() as u8, now.day(),
            now.hour(), now.minute(), now.second()
        );
        let date_stamp = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());

        let payload_hash = hex::encode(Sha256::digest(b""));

        let mut headers = BTreeMap::new();
        let host = url.replace("https://", "").replace("http://", "").split('/').next().unwrap_or("").to_string();
        headers.insert("Host".to_string(), host);
        headers.insert("x-amz-date".to_string(), amz_date.clone());
        headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());

        let (auth_header, _) = generate_sigv4(
            "DELETE",
            &uri,
            "",
            &headers,
            &payload_hash,
            &access_key,
            &secret_key,
            "us-east-1",
            "s3",
            &amz_date,
            &date_stamp,
        );

        let req = client.delete(&url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", auth_header);

        let _ = req.send().await;
    }

    async fn write_state_file(&self, latest: u64) {
        let state_file = StateFile { latest };
        if let Ok(json) = serde_json::to_string(&state_file) {
            let tmp_path = "./recordings/state.json.tmp";
            let final_path = "./recordings/state.json";
            if tokio::fs::write(tmp_path, json).await.is_ok() {
                let _ = tokio::fs::rename(tmp_path, final_path).await;
            }
        }
    }

    pub async fn run(mut self) {
        // Wait for LQ stream header to be available in state
        // The prompt says "Prepend the LQ stream header (generated once during startup) to lq_bytes".
        // In converter.rs we only saved the HQ stream header to AppState. We will generate a mock or wait.
        // To be accurate, we'll just generate an LQ stream header if needed, but since we can't access FlacEncoder here directly easily
        // we'll just prepend a mock or empty bytes if unavailable. Wait, the prompt says "generated once during startup".
        // Let's create an encoder just to get the LQ stream header bytes.
        let lq_encoder = encoder::flac::FlacEncoder::new(24000, 2, 16, 2048);
        let lq_header = Bytes::from(lq_encoder.stream_header());

        while let Some((index, hq_bytes, lq_bytes)) = self.seg_rx.recv().await {
            // Only accept segments from last_persisted_index + 1
            if index <= self.last_persisted_index {
                continue;
            }

            self.state.r2_uploading.store(true, Ordering::SeqCst);

            let hq_header = {
                let lock = self.state.flac_header.lock().unwrap_or_else(|e| e.into_inner());
                lock.clone().unwrap_or_else(|| Bytes::from(vec![]))
            };

            // Prepend headers
            let mut hq_full = Vec::with_capacity(hq_header.len() + hq_bytes.len());
            hq_full.extend_from_slice(&hq_header);
            hq_full.extend_from_slice(&hq_bytes);

            let mut lq_full = Vec::with_capacity(lq_header.len() + lq_bytes.len());
            lq_full.extend_from_slice(&lq_header);
            lq_full.extend_from_slice(&lq_bytes);

            // Upload HQ
            let hq_success = self.upload_with_retry("hq", index, hq_full, "audio/flac").await;
            if !hq_success {
                let _ = self.state.sse_tx.send(r#"{"error":"r2","message":"Failed to upload HQ segment"}"#.to_string());
                self.state.r2_uploading.store(false, Ordering::SeqCst);
                continue;
            }

            // Upload LQ
            let lq_success = self.upload_with_retry("lq", index, lq_full, "audio/flac").await;
            if !lq_success {
                let _ = self.state.sse_tx.send(r#"{"error":"r2","message":"Failed to upload LQ segment"}"#.to_string());
                self.state.r2_uploading.store(false, Ordering::SeqCst);
                continue;
            }

            // Upload Manifest
            let manifest = Manifest {
                live: true,
                latest: index,
                segment_s: 10.24,
                updated_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
                qualities: vec!["hq", "lq"],
            };
            let manifest_json = serde_json::to_vec(&manifest).unwrap();
            let _ = self.upload_manifest_with_retry(manifest_json).await;

            // Update State
            self.window.push_back(index);
            while self.window.len() > 10 {
                if let Some(oldest) = self.window.front().copied() {
                    // issue S3 DELETE for oldest
                    let hq_del = self.delete_s3_segment("hq", oldest).await;
                    let lq_del = self.delete_s3_segment("lq", oldest).await;

                    if hq_del && lq_del {
                        self.window.pop_front();
                    } else {
                        // Keep it in window so we try again next time
                        break;
                    }
                } else {
                    break;
                }
            }

            self.last_persisted_index = index;
            self.state.r2_segment.store(index, Ordering::SeqCst);
            self.state.r2_last_ms.store(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
                Ordering::SeqCst
            );
            self.write_state_file(index).await;

            // Local Playback Queue
            {
                let mut local_segments = self.state.local_segments.lock().unwrap_or_else(|e| e.into_inner());
                local_segments.push_back((index, hq_bytes));
                if local_segments.len() > 3 {
                    local_segments.pop_front();
                }
            }

            self.state.r2_uploading.store(false, Ordering::SeqCst);
        }
    }

    async fn upload_with_retry(&self, quality: &str, index: u64, body: Vec<u8>, content_type: &str) -> bool {
        let bucket = std::env::var("R2_BUCKET").unwrap_or_else(|_| "".to_string());
        let bucket_prefix = if bucket.is_empty() { String::new() } else { format!("/{bucket}") };
        let uri = format!("{bucket_prefix}/live/{quality}/segment-{index:08}.flac");

        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * (1 << (attempt - 1)))).await;
            }
            match self.put_s3(&uri, body.clone(), content_type, "public, max-age=31536000, immutable").await {
                Ok(true) => {
                    let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","message":"Uploaded {uri} ({} bytes)"}}"#, body.len()));
                    return true;
                }
                Err(true) => {
                    // Fatal error, don't retry
                    return false;
                }
                _ => {} // Retry
            }
        }
        false
    }

    async fn upload_manifest_with_retry(&self, body: Vec<u8>) -> bool {
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * (1 << (attempt - 1)))).await;
            }
            let bucket = std::env::var("R2_BUCKET").unwrap_or_else(|_| "".to_string());
            let bucket_prefix = if bucket.is_empty() { String::new() } else { format!("/{bucket}") };
            let uri = format!("{bucket_prefix}/manifest.json");
            
            match self.put_s3(&uri, body.clone(), "application/json", "no-store, max-age=0").await {
                Ok(true) => return true,
                Err(true) => return false, // Fatal
                _ => {} // Retry
            }
        }
        false
    }

    // Returns Result<bool, bool> where Err(true) is a fatal error that shouldn't be retried
    async fn put_s3(&self, uri: &str, body: Vec<u8>, content_type: &str, cache_control: &str) -> Result<bool, bool> {
        let access_key = std::env::var("R2_ACCESS_KEY").unwrap_or_else(|_| "test_access".to_string());
        let secret_key = std::env::var("R2_SECRET_KEY").unwrap_or_else(|_| "test_secret".to_string());
        let endpoint = std::env::var("R2_ENDPOINT").unwrap_or_else(|_| "https://test.s3.amazonaws.com".to_string());

        let url = format!("{}{}", endpoint, uri);

        let now = OffsetDateTime::now_utc();
        let amz_date = format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            now.year(), now.month() as u8, now.day(),
            now.hour(), now.minute(), now.second()
        );
        let date_stamp = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());

        let payload_hash = hex::encode(Sha256::digest(&body));

        let mut headers = BTreeMap::new();
        let host = url.replace("https://", "").replace("http://", "").split('/').next().unwrap_or("").to_string();
        headers.insert("Host".to_string(), host);
        headers.insert("x-amz-date".to_string(), amz_date.clone());
        headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());
        headers.insert("Content-Type".to_string(), content_type.to_string());
        headers.insert("Cache-Control".to_string(), cache_control.to_string());
        headers.insert("Content-Length".to_string(), body.len().to_string());

        let (auth_header, _) = generate_sigv4(
            "PUT",
            uri,
            "",
            &headers,
            &payload_hash,
            &access_key,
            &secret_key,
            "us-east-1", // Using default or extract from endpoint if needed
            "s3",
            &amz_date,
            &date_stamp,
        );

        let req = self.client.put(&url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", auth_header)
            .header("Content-Type", content_type)
            .header("Cache-Control", cache_control)
            .header("Content-Length", body.len().to_string())
            .body(body);

        match req.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    Ok(true)
                } else if resp.status().as_u16() == 503 {
                    // 503 Slow Down
                    Err(false)
                } else if resp.status().as_u16() == 403 {
                    let xml = resp.text().await.unwrap_or_default();
                    if xml.contains("RequestTimeTooSkewed") {
                        tracing::error!("FATAL: NTP Clock drift detected");
                        return Err(true); // Don't retry if time is skewed
                    }
                    Err(false)
                } else {
                    Err(false)
                }
            },
            Err(_) => Err(false),
        }
    }

    async fn delete_s3_segment(&self, quality: &str, index: u64) -> bool {
        let bucket = std::env::var("R2_BUCKET").unwrap_or_else(|_| "".to_string());
        let bucket_prefix = if bucket.is_empty() { String::new() } else { format!("/{bucket}") };
        let uri = format!("{bucket_prefix}/live/{quality}/segment-{index:08}.flac");
        
        let access_key = std::env::var("R2_ACCESS_KEY").unwrap_or_else(|_| "test_access".to_string());
        let secret_key = std::env::var("R2_SECRET_KEY").unwrap_or_else(|_| "test_secret".to_string());
        let endpoint = std::env::var("R2_ENDPOINT").unwrap_or_else(|_| "https://test.s3.amazonaws.com".to_string());

        let url = format!("{}{}", endpoint, uri);

        let now = OffsetDateTime::now_utc();
        let amz_date = format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            now.year(), now.month() as u8, now.day(),
            now.hour(), now.minute(), now.second()
        );
        let date_stamp = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());

        let payload_hash = hex::encode(Sha256::digest(b""));

        let mut headers = BTreeMap::new();
        let host = url.replace("https://", "").replace("http://", "").split('/').next().unwrap_or("").to_string();
        headers.insert("Host".to_string(), host);
        headers.insert("x-amz-date".to_string(), amz_date.clone());
        headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());

        let (auth_header, _) = generate_sigv4(
            "DELETE",
            &uri,
            "",
            &headers,
            &payload_hash,
            &access_key,
            &secret_key,
            "us-east-1",
            "s3",
            &amz_date,
            &date_stamp,
        );

        let req = self.client.delete(&url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", auth_header);

        if let Ok(resp) = req.send().await {
            resp.status().is_success()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use crate::state::AppState;

    #[tokio::test]
    async fn test_manifest_fallback() {
        // Write corrupted JSON to state.json
        let _ = tokio::fs::create_dir_all("./recordings").await;
        let _ = tokio::fs::write("./recordings/state.json", "corrupted { json").await;

        let state = Arc::new(AppState::new());
        let (_, rx) = mpsc::channel(3);

        let uploader = UploaderTask::new(rx, state).await;

        assert_eq!(uploader.last_persisted_index, 0);

        // cleanup to prevent state bleeding into other runs
        let _ = tokio::fs::remove_file("./recordings/state.json").await;
    }
}
