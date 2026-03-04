use std::sync::Arc;
use tokio::sync::mpsc;
use crate::state::AppState;
use capture::capture::CaptureLoop;
use capture::device::Device;

use encoder::flac::FlacEncoder;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tokio_util::sync::CancellationToken;

pub struct RecorderTask {
    pcm_tx: mpsc::Sender<Arc<Vec<i32>>>,
    state: Arc<AppState>,
    local_archive_dir: PathBuf,
    token: CancellationToken,
}

impl RecorderTask {
    pub fn new(
        pcm_tx: mpsc::Sender<Arc<Vec<i32>>>,
        state: Arc<AppState>,
        local_archive_dir: PathBuf,
        token: CancellationToken,
    ) -> Self {
        RecorderTask {
            pcm_tx,
            state,
            local_archive_dir,
            token,
        }
    }

    pub async fn run(self) -> std::io::Result<()> {
        let mut capture_loop: Option<CaptureLoop> = None;
        let mut current_device_path = String::new();
        
        // For local archive file
        let mut archive_file: Option<File> = None;
        let archive_encoder = FlacEncoder::new(48000, 2, 24, 4096);
        let mut frames_in_file = 0u64;
        let mut file_frame_number = 0u64;
        let frames_per_hour = 48000 * 60 * 60; // 172,800,000 frames

        loop {
            if self.token.is_cancelled() {
                break Ok(());
            }

            // Check current desired state
            let desired_device = { self.state.selected_device.lock().unwrap().clone() };
            let should_stream = self.state.streaming.load(Ordering::SeqCst);
            
            // Reconfigure if device changed or we stopped streaming
            if !should_stream || desired_device != current_device_path {
                if capture_loop.is_some() {
                    let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","message":"Closing capture device: {}"}}"#, current_device_path));
                    capture_loop = None;
                    current_device_path = String::new();
                }
            }

            if should_stream && capture_loop.is_none() {
                // Try to open
                let is_mock = desired_device == "mock_device";
                let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","message":"Opening capture device: {}"}}"#, desired_device));

                if is_mock {
                    current_device_path = desired_device;
                    // capture_loop remains None for mock
                } else {
                    match Device::open(&desired_device) {
                        Ok(device) => {
                            device.prepare();
                            match CaptureLoop::new(device.raw_fd()) {
                                Ok(cl) => {
                                    capture_loop = Some(cl);
                                    current_device_path = desired_device;
                                    let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","message":"Success opening {}"}}"#, current_device_path));
                                },
                                Err(e) => {
                                    let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","error":true,"message":"CaptureLoop error: {}"}}"#, e));
                                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                    continue;
                                }
                            }
                        },
                        Err(e) => {
                            let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","error":true,"message":"Device open error: {}"}}"#, e));
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            continue;
                        }
                    }
                }
            }
            
            if !should_stream {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }

            // Need a rotation?
            if archive_file.is_none() || frames_in_file >= frames_per_hour {
                if let Some(file) = archive_file.take() {
                    let _ = file.sync_all(); // Fsync old file
                }

                // Open new timestamped file
                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                let filename = format!("archive_{}.flac", ts);
                let filepath = self.local_archive_dir.join(&filename);

                {
                    let mut rp = self.state.recording_path.lock().unwrap_or_else(|e| e.into_inner());
                    *rp = filepath.to_string_lossy().to_string();
                }

                let mut new_file = File::create(&filepath)?;
                let header = archive_encoder.stream_header();
                new_file.write_all(&header)?;

                archive_file = Some(new_file);
                frames_in_file = 0;
                file_frame_number = 0; // reset FLAC frame numbering for the new file
            }

            // Wait for ALSA readable event, or generate mock silence
            let (mut pcm_data, overrun) = if let Some(loop_ref) = &capture_loop {
                match loop_ref.read_period().await {
                    Ok(res) => res,
                    Err(e) => {
                        let errno = e.raw_os_error().unwrap_or(0);
                        if errno == libc::ENODEV {
                            let _ = self.state.sse_tx.send(r#"{"type":"log","error":true,"message":"Device disconnected (ENODEV)"}"#.to_string());
                            capture_loop = None;
                            current_device_path = String::new(); // force reopen
                            continue;
                        } else {
                            let _ = self.state.sse_tx.send(format!(r#"{{"type":"log","error":true,"message":"ALSA read err: {}"}}"#, e));
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                }
            } else {
                // Mock device loop: Wait ~85ms for 4096 samples at 48kHz
                tokio::time::sleep(std::time::Duration::from_millis(85)).await;
                let silence = vec![0i32; 8192]; // 4096 frames * 2 channels
                (silence, false)
            };

            if overrun {
                self.state.overruns.fetch_add(1, Ordering::Relaxed);
                let _ = self.state.sse_tx.send(r#"{"type":"log","error":true,"message":"Buffer overrun (XRUN)"}"#.to_string());
            }
            
            let frames_read = pcm_data.len() / 2;
            
            // Channel routing
            let desired_channel = { self.state.selected_channel.lock().unwrap().clone() };
            if desired_channel == "left" {
                for i in (0..pcm_data.len()).step_by(2) {
                    pcm_data[i+1] = pcm_data[i];
                }
            } else if desired_channel == "right" {
                for i in (0..pcm_data.len()).step_by(2) {
                    pcm_data[i] = pcm_data[i+1];
                }
            }

            // Calculate VU values
            let mut max_l = 0;
            let mut max_r = 0;
            for i in (0..pcm_data.len()).step_by(2) {
                let l = pcm_data[i].abs();
                let r = pcm_data[i+1].abs();
                if l > max_l { max_l = l; }
                if r > max_r { max_r = r; }
            }
            self.state.vu_left.store(max_l, Ordering::Relaxed);
            self.state.vu_right.store(max_r, Ordering::Relaxed);

            // Encode to local archive
            let flac_frame = archive_encoder.encode_frame(&pcm_data, file_frame_number);
            if let Some(f) = &mut archive_file {
                f.write_all(&flac_frame)?;
                self.state.recording_bytes.fetch_add(flac_frame.len() as u64, Ordering::Relaxed);
            }

            file_frame_number += 1;
            frames_in_file += frames_read as u64;

            // Send ARC wrapper to ConverterTask
            let arc_pcm = Arc::new(pcm_data);
            if let Err(_) = self.pcm_tx.try_send(arc_pcm) {
                eprintln!("WARN: pcm_tx full, dropping PCM block to prevent block loop");
            }
        }
    }
}
