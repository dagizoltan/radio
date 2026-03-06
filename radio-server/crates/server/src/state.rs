use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
use std::sync::Mutex;
use bytes::Bytes;
use tokio::sync::broadcast;

pub struct AppState {
    pub streaming: AtomicBool,
    pub vu_left: AtomicI32,
    pub vu_right: AtomicI32,
    pub stream_vu_left: AtomicI32,
    pub stream_vu_right: AtomicI32,
    pub recording_path: Mutex<String>,
    pub recording_start: AtomicU64,
    pub recording_bytes: AtomicU64,
    pub r2_segment: AtomicU64,
    pub r2_last_ms: AtomicU64,
    pub r2_uploading: AtomicBool,
    pub local_segments: Mutex<VecDeque<(u64, Bytes)>>,
    pub flac_header: Mutex<Option<Bytes>>,
    pub sse_tx: broadcast::Sender<String>,
    pub overruns: AtomicU64,
    pub selected_device: Mutex<String>,
    pub selected_channel: Mutex<String>,
    pub waveform: Mutex<Vec<i16>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let (sse_tx, _) = broadcast::channel(16);
        AppState {
            streaming: AtomicBool::new(false),
            vu_left: AtomicI32::new(0),
            vu_right: AtomicI32::new(0),
            stream_vu_left: AtomicI32::new(0),
            stream_vu_right: AtomicI32::new(0),
            recording_path: Mutex::new(String::new()),
            recording_start: AtomicU64::new(0),
            recording_bytes: AtomicU64::new(0),
            r2_segment: AtomicU64::new(0),
            r2_last_ms: AtomicU64::new(0),
            r2_uploading: AtomicBool::new(false),
            local_segments: Mutex::new(VecDeque::new()),
            flac_header: Mutex::new(None),
            sse_tx,
            overruns: AtomicU64::new(0),
            selected_device: Mutex::new("mock_device".to_string()),
            selected_channel: Mutex::new("stereo".to_string()),
            waveform: Mutex::new(vec![0; 128]),
        }
    }
}
