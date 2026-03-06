use axum::{
    extract::{Path, State},
    response::{sse::{Event, Sse}, Html, Response},
    routing::{get, post},
    Router, Json,
};
use bytes::Bytes;
use reqwest::StatusCode;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::{Stream, StreamExt};
use tower_http::cors::{Any, CorsLayer};
use crate::state::AppState;
use capture::discovery::get_available_devices;
use serde::{Deserialize, Serialize};

use tokio_util::sync::CancellationToken;

pub async fn run_server(state: Arc<AppState>, token: CancellationToken) {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(index))
        .route("/events", get(sse_handler))
        .route("/local/{id}", get(local_segment))
        .route("/start", post(start_stream))
        .route("/stop", post(stop_stream))
        .route("/metrics", get(metrics_handler))
        .route("/devices", get(list_devices))
        .route("/settings", post(update_settings))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    tracing::info!("Monitor UI listening on http://0.0.0.0:8080");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            token.cancelled().await;
        })
        .await
        .unwrap();
}

async fn index() -> Html<&'static str> {
    Html(include_str!("monitor.html"))
}

async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sse_tx.subscribe();

    // Convert BroadcastReceiver into a Stream
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx);

    // Map the stream events
    let mapped_stream = stream.map(|msg| match msg {
        Ok(s) => Ok(Event::default().data(s)),
        Err(_) => Ok(Event::default().event("error").data("stream skipped")),
    });

    // We can also create a keep-alive stream that ticks periodically
    let _keepalive_stream = tokio_stream::wrappers::IntervalStream::new(
        tokio::time::interval(std::time::Duration::from_secs(30))
    ).map(|_| {
        Ok::<Event, Infallible>(Event::default().event("ping").data("keepalive"))
    });

    // To merge them we could use `tokio_stream::StreamExt::merge` but let's stick to the ping via axum::response::sse::KeepAlive if needed,
    // or manually merging streams. For simplicity, we can spawn a background task pushing metrics to sse_tx directly
    // or just return the broadcast stream as SSE.

    // Axum Sse has built in keepalive
    Sse::new(mapped_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(30))
            .text("keepalive")
    )
}

async fn local_segment(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let segments = state.local_segments.lock().unwrap_or_else(|e| e.into_inner());

    // Find the segment with index == id
    let hq_bytes = segments.iter().find(|(index, _)| *index == id).map(|(_, bytes)| bytes.clone());

    if let Some(audio_bytes) = hq_bytes {
        // Prepend FLAC header
        let header = {
            let lock = state.flac_header.lock().unwrap_or_else(|e| e.into_inner());
            lock.clone().unwrap_or_else(|| Bytes::from(vec![]))
        };

        let mut full_payload = Vec::with_capacity(header.len() + audio_bytes.len());
        full_payload.extend_from_slice(&header);
        full_payload.extend_from_slice(&audio_bytes);

        let mut response = Response::new(axum::body::Body::from(full_payload));
        response.headers_mut().insert(reqwest::header::CONTENT_TYPE, "audio/flac".parse().unwrap());
        Ok(response)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn start_stream(State(state): State<Arc<AppState>>) -> StatusCode {
    state.streaming.store(true, std::sync::atomic::Ordering::SeqCst);
    StatusCode::OK
}

async fn stop_stream(State(state): State<Arc<AppState>>) -> StatusCode {
    state.streaming.store(false, std::sync::atomic::Ordering::SeqCst);
    StatusCode::OK
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> String {
    // Prometheus format
    let overruns = state.overruns.load(std::sync::atomic::Ordering::Relaxed);
    let put_latency = 0.0; // Hardcoded for now until full metric struct is injected
    let rec_bytes = state.recording_bytes.load(std::sync::atomic::Ordering::Relaxed);

    format!(
        "# HELP radio_capture_overruns_total Total ALSA capture buffer overruns.\n\
         # TYPE radio_capture_overruns_total counter\n\
         radio_capture_overruns_total {}\n\
         # HELP radio_s3_put_latency_seconds Latency of S3 PUT requests.\n\
         # TYPE radio_s3_put_latency_seconds summary\n\
         radio_s3_put_latency_seconds {}\n\
         # HELP radio_recording_bytes_total Current local archive file size in bytes.\n\
         # TYPE radio_recording_bytes_total gauge\n\
         radio_recording_bytes_total {}\n",
        overruns,
        put_latency,
        rec_bytes
    )
}

#[derive(Serialize)]
struct DeviceDesc {
    id: String,
    label: String,
}

async fn list_devices() -> Json<Vec<DeviceDesc>> {
    let devices = get_available_devices()
        .into_iter()
        .map(|(id, label)| DeviceDesc { id, label })
        .collect();
    Json(devices)
}

#[derive(Deserialize)]
struct SettingsPayload {
    device: String,
    channel: String, // "stereo", "left", "right"
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SettingsPayload>,
) -> StatusCode {
    {
        let mut dev = state.selected_device.lock().unwrap();
        *dev = payload.device;
    }
    {
        let mut ch = state.selected_channel.lock().unwrap();
        *ch = payload.channel;
    }
    let _ = state.sse_tx.send(r#"{"type":"log","message":"Settings updated"}"#.to_string());
    
    // Stop the stream momentarily. The recorder loop will pick up the device changes.
    // Let's just update the settings, and let the user Start Stream again.
    
    StatusCode::OK
}
