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
    let state = Arc::new(AppState::new());
    let token = CancellationToken::new();

    let (pcm_tx, pcm_rx) = mpsc::channel(16);
    let (seg_tx, seg_rx) = mpsc::channel(3);

    let local_archive_dir = PathBuf::from("./archive");
    std::fs::create_dir_all(&local_archive_dir)?;

    let recorder = RecorderTask::new(pcm_tx, state.clone(), local_archive_dir, token.clone());
    let converter = ConverterTask::new(pcm_rx, seg_tx, state.clone());
    let uploader = UploaderTask::new(seg_rx, state.clone()).await;

    let recorder_handle = tokio::spawn(async move {
        if let Err(e) = recorder.run().await {
            eprintln!("Recorder error: {:?}", e);
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

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    tokio::select! {
        _ = sigterm.recv() => {
            println!("Received SIGTERM, shutting down...");
        }
        _ = sigint.recv() => {
            println!("Received SIGINT, shutting down...");
        }
    }

    token.cancel();

    // Await all with timeout
    let timeout_duration = Duration::from_secs(25);
    let result = tokio::time::timeout(timeout_duration, async {
        let _ = tokio::join!(recorder_handle, converter_handle, uploader_handle, http_handle);
    }).await;

    if result.is_err() {
        eprintln!("Shutdown timed out after 25 seconds, forcefully aborting...");
        std::process::exit(1);
    }

    Ok(())
}
