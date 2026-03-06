use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use server::state::AppState;
use server::recorder::RecorderTask;
use server::converter::ConverterTask;
use server::uploader::UploaderTask;
use server::http::run_server;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let state = Arc::new(AppState::new());
    let token = CancellationToken::new();

    let (pcm_tx, pcm_rx) = mpsc::channel(16);
    let (seg_tx, seg_rx) = mpsc::channel(3);

    let local_archive_dir = PathBuf::from("./archive");
    std::fs::create_dir_all(&local_archive_dir)?;

    let recorder = RecorderTask::new(pcm_tx, state.clone(), local_archive_dir, token.clone());
    let uploader = UploaderTask::new(seg_rx, state.clone()).await;
    let converter = ConverterTask::new(pcm_rx, seg_tx, state.clone());

    let recorder_handle = tokio::spawn(async move {
        if let Err(e) = recorder.run().await {
            tracing::error!("Recorder error: {:?}", e);
        }
    });

    let converter_handle = tokio::spawn(async move {
        converter.run().await;
    });

    let uploader_handle = tokio::spawn(async move {
        uploader.run().await;
    });

    let http_state = state.clone();
    let http_token = token.clone();
    let http_handle = tokio::spawn(async move {
        run_server(http_state, http_token).await;
    });

    // Periodic metrics push → SSE (every 500ms)
    let metrics_state = state.clone();
    let metrics_token = token.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = metrics_token.cancelled() => break,
            }
            let streaming = metrics_state.streaming.load(std::sync::atomic::Ordering::Relaxed);
            let vu_left   = metrics_state.vu_left.load(std::sync::atomic::Ordering::Relaxed);
            let vu_right  = metrics_state.vu_right.load(std::sync::atomic::Ordering::Relaxed);
            let stream_vu_left   = metrics_state.stream_vu_left.load(std::sync::atomic::Ordering::Relaxed);
            let stream_vu_right  = metrics_state.stream_vu_right.load(std::sync::atomic::Ordering::Relaxed);
            let r2_seg    = metrics_state.r2_segment.load(std::sync::atomic::Ordering::Relaxed);
            let overruns  = metrics_state.overruns.load(std::sync::atomic::Ordering::Relaxed);
            let uploading = metrics_state.r2_uploading.load(std::sync::atomic::Ordering::Relaxed);
            let rec_bytes = metrics_state.recording_bytes.load(std::sync::atomic::Ordering::Relaxed);
            let rec_start = metrics_state.recording_start.load(std::sync::atomic::Ordering::Relaxed);
            let rec_path  = metrics_state.recording_path.lock()
                .unwrap_or_else(|e| e.into_inner()).clone();
            
            let wf = {
                let mut wf_lock = metrics_state.waveform.lock().unwrap();
                let current_wf = wf_lock.clone();
                // Clear the waveform after reading to prevent stale data
                wf_lock.fill(0);
                current_wf
            };

            let msg = serde_json::json!({
                "type": "metrics",
                "streaming": streaming,
                "vu_left": vu_left,
                "vu_right": vu_right,
                "stream_vu_left": stream_vu_left,
                "stream_vu_right": stream_vu_right,
                "r2_segment": r2_seg,
                "overruns": overruns,
                "uploading": uploading,
                "recording_bytes": rec_bytes,
                "recording_start": rec_start,
                "recording_path": rec_path,
                "waveform": wf
            });

            let msg_str = serde_json::to_string(&msg).unwrap();
            let _ = metrics_state.sse_tx.send(msg_str);
        }
    });

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, shutting down...");
        }
        _ = sigint.recv() => {
            tracing::info!("Received SIGINT, shutting down...");
        }
    }

    token.cancel();

    // Await all with timeout
    let timeout_duration = Duration::from_secs(25);
    let result = tokio::time::timeout(timeout_duration, async {
        let _ = tokio::join!(recorder_handle, converter_handle, uploader_handle, http_handle);
    }).await;

    if result.is_err() {
        tracing::error!("Shutdown timed out after 25 seconds, forcefully aborting...");
        std::process::exit(1);
    }

    Ok(())
}
