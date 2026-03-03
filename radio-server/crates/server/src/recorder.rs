use std::sync::Arc;
use tokio::sync::mpsc;
use crate::state::AppState;
use capture::capture::CaptureLoop;
use capture::device::Device;
use capture::discovery::discover_device;
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
        let device_path = discover_device();
        println!("Opening capture device: {}", device_path);

        let device = Device::open(&device_path);
        device.prepare();

        let capture_loop = CaptureLoop::new(device.raw_fd())?;

        // For local archive file
        let mut archive_file: Option<File> = None;
        let archive_encoder = FlacEncoder::new(48000, 2, 24, 4096);
        let mut frames_in_file = 0u64;
        let mut file_frame_number = 0u64;
        let frames_per_hour = 48000 * 60 * 60; // 172,800,000 frames

        self.state.streaming.store(true, Ordering::SeqCst);

        loop {
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
                    let mut rp = self.state.recording_path.lock().unwrap();
                    *rp = filepath.to_string_lossy().to_string();
                }

                let mut new_file = File::create(&filepath)?;
                let header = archive_encoder.stream_header();
                new_file.write_all(&header)?;

                archive_file = Some(new_file);
                frames_in_file = 0;
                file_frame_number = 0; // reset FLAC frame numbering for the new file
            }

            // Wait for ALSA readable event
            let (pcm_data, overrun) = match capture_loop.read_period().await {
                Ok(res) => res,
                Err(e) => {
                    let errno = e.raw_os_error().unwrap_or(0);
                    if errno == libc::ENODEV {
                        eprintln!("RecorderTask: Device disconnected (ENODEV). Initiating shutdown.");
                        self.token.cancel();
                        return Err(e);
                    } else {
                        return Err(e);
                    }
                }
            };

            if overrun {
                self.state.overruns.fetch_add(1, Ordering::Relaxed);
            }
            let frames_read = pcm_data.len() / 2;

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
