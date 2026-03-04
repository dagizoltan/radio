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
        let (device_path, channels) = discover_device();
        println!("Opening capture device: {} with {} channels", device_path, channels);

        let mut capture_loop = None;
        let mut mock_mode = false;
        let mut _active_device = None;

        if device_path == "mock_device" {
            println!("Mock device mode enabled. Generating silence.");
            mock_mode = true;
        } else {
            let device = Device::open(&device_path, channels);
            device.prepare();

            // The input channels to use for Left and Right (0-indexed).
            // For a 4-channel device, maybe 0 and 1, or 2 and 3.
            let left_channel = std::env::var("AUDIO_LEFT_CHANNEL").unwrap_or_else(|_| "0".to_string()).parse().unwrap_or(0);
            let right_channel = std::env::var("AUDIO_RIGHT_CHANNEL").unwrap_or_else(|_| "1".to_string()).parse().unwrap_or(1);

            println!("Using channel {} for Left, channel {} for Right", left_channel, right_channel);

            capture_loop = Some(CaptureLoop::new(device.raw_fd(), channels, left_channel, right_channel)?);
            _active_device = Some(device);
        }

        // For local archive file
        let mut archive_file: Option<File> = None;
        let archive_encoder = FlacEncoder::new(48000, 2, 24, 4096);
        let mut frames_in_file = 0u64;
        let mut file_frame_number = 0u64;
        let frames_per_hour = 48000 * 60 * 60; // 172,800,000 frames

        self.state.streaming.store(true, Ordering::SeqCst);

        let mut mock_phase_l: f32 = 0.0;
        let mut mock_phase_r: f32 = 0.0;

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

            // Wait for ALSA readable event or generate mock data
            let (pcm_data, overrun) = if mock_mode {
                tokio::time::sleep(std::time::Duration::from_millis(85)).await; // Approx 4096 frames at 48kHz
                // Generate a real sine wave for mock testing: 440 Hz (L) and 880 Hz (R)
                let mut mock_pcm = Vec::with_capacity(4096 * 2);
                let freq_l = 440.0;
                let freq_r = 880.0;
                let sample_rate = 48000.0;
                // Use a reasonable amplitude for 24-bit audio (max is ~8.38M, we use 1M so it's not deafening)
                let amplitude = 1_000_000.0;

                for _ in 0..4096 {
                    let sample_l = (mock_phase_l * std::f32::consts::TAU).sin() * amplitude;
                    let sample_r = (mock_phase_r * std::f32::consts::TAU).sin() * amplitude;

                    mock_pcm.push(sample_l as i32); // L
                    mock_pcm.push(sample_r as i32); // R

                    mock_phase_l = (mock_phase_l + freq_l / sample_rate).fract();
                    mock_phase_r = (mock_phase_r + freq_r / sample_rate).fract();
                }
                (mock_pcm, false)
            } else {
                match capture_loop.as_ref().unwrap().read_period().await {
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
