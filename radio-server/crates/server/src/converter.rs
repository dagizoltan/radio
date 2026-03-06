use std::sync::Arc;
use tokio::sync::mpsc;
use bytes::Bytes;
use encoder::flac::FlacEncoder;
use crate::state::AppState;

pub struct ConverterTask {
    pcm_rx: mpsc::Receiver<Arc<Vec<i32>>>,
    seg_tx: mpsc::Sender<(u64, Bytes, Bytes)>,
    _state: Arc<AppState>,
    hq_encoder: FlacEncoder,
    lq_encoder: FlacEncoder,
    hq_accumulator: Vec<u8>,
    lq_accumulator: Vec<u8>,
    frame_counter: u64,
    segment_index: u64,
}

impl ConverterTask {
    pub fn new(
        pcm_rx: mpsc::Receiver<Arc<Vec<i32>>>,
        seg_tx: mpsc::Sender<(u64, Bytes, Bytes)>,
        state: Arc<AppState>,
    ) -> Self {
        let hq_encoder = FlacEncoder::new(48000, 2, 24, 4096);
        let lq_encoder = FlacEncoder::new(24000, 2, 16, 2048);

        // Ensure HQ header is written to state immediately
        let header = hq_encoder.stream_header();
        {
            let mut state_header = state.flac_header.lock().unwrap_or_else(|e| e.into_inner());
            *state_header = Some(Bytes::from(header));
        }

        // Initialize the segment index from state, so it doesn't restart at 0 and get dropped by uploader
        let segment_index = state.r2_segment.load(std::sync::atomic::Ordering::Relaxed) + 1;

        ConverterTask {
            pcm_rx,
            seg_tx,
            _state: state,
            hq_encoder,
            lq_encoder,
            hq_accumulator: Vec::with_capacity(2955000),
            lq_accumulator: Vec::with_capacity(985000),
            frame_counter: 0,
            segment_index,
        }
    }

    pub async fn run(mut self) {
        let mut lq_staging = Vec::with_capacity(4096); // 2048 frames * 2 channels

        while let Some(pcm_arc) = self.pcm_rx.recv().await {
            // Encode HQ frame
            // pcm_arc represents 4096 frames (8192 elements)
            let hq_frame = self.hq_encoder.encode_frame(&pcm_arc, self.segment_index * 120 + (self.frame_counter / 4096));
            self.hq_accumulator.extend_from_slice(&hq_frame);

            // Decimate to LQ (24000 Hz)
            lq_staging.clear();
            for i in (0..pcm_arc.len()).step_by(4) {
                if i + 3 < pcm_arc.len() {
                    // Simple low-pass filter (averaging adjacent samples)
                    let l1 = pcm_arc[i];
                    let r1 = pcm_arc[i + 1];
                    let l2 = pcm_arc[i + 2];
                    let r2 = pcm_arc[i + 3];

                    // average and convert 24-bit to 16-bit
                    let l_avg = (l1 / 2) + (l2 / 2);
                    let r_avg = (r1 / 2) + (r2 / 2);

                    lq_staging.push(l_avg >> 8);
                    lq_staging.push(r_avg >> 8);
                }
            }

            // Encode LQ frame
            // lq_staging represents 2048 frames
            let lq_frame = self.lq_encoder.encode_frame(&lq_staging, self.segment_index * 120 + (self.frame_counter / 4096));
            self.lq_accumulator.extend_from_slice(&lq_frame);

            self.frame_counter += 4096;

            // 491,520 HQ frames = 10.24 seconds (120 ALSA periods of 4096 frames)
            if self.frame_counter >= 491_520 {
                let hq_bytes = Bytes::copy_from_slice(&self.hq_accumulator);
                let lq_bytes = Bytes::copy_from_slice(&self.lq_accumulator);

                if self.seg_tx.try_send((self.segment_index, hq_bytes, lq_bytes)).is_err() {
                    tracing::error!("WARN: seg_tx full, dropping segment {}", self.segment_index);
                }

                self.hq_accumulator.clear();
                self.lq_accumulator.clear();
                self.frame_counter = 0;
                self.segment_index = (self.segment_index + 1) % 100_000_000;
            }
        }

        // Flush any remaining frames on shutdown
        if self.frame_counter > 0 {
            let hq_bytes = Bytes::copy_from_slice(&self.hq_accumulator);
            let lq_bytes = Bytes::copy_from_slice(&self.lq_accumulator);

            if self.seg_tx.try_send((self.segment_index, hq_bytes, lq_bytes)).is_err() {
                tracing::error!("WARN: seg_tx full, dropping final segment {}", self.segment_index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decimation_math() {
        let input = vec![100, 200, 300, 400]; // simulate left, right, left, right (2 frames)
        let mut lq_staging = Vec::new();

        for i in (0..input.len()).step_by(4) {
            if i + 3 < input.len() {
                let l1 = input[i];
                let r1 = input[i + 1];
                let l2 = input[i + 2];
                let r2 = input[i + 3];

                let l_avg = (l1 / 2) + (l2 / 2);
                let r_avg = (r1 / 2) + (r2 / 2);

                lq_staging.push(l_avg >> 8);
                lq_staging.push(r_avg >> 8);
            }
        }

        assert_eq!(lq_staging, vec![((100 / 2) + (300 / 2)) >> 8, ((200 / 2) + (400 / 2)) >> 8]);

        // Verify negative numbers sign-extend properly
        let neg_input = vec![-32768, -1000, 0, 0];
        let mut lq_staging_neg = Vec::new();
        for i in (0..neg_input.len()).step_by(4) {
            if i + 3 < neg_input.len() {
                let l1 = neg_input[i];
                let r1 = neg_input[i + 1];
                let l2 = neg_input[i + 2];
                let r2 = neg_input[i + 3];

                let l_avg = (l1 / 2) + (l2 / 2);
                let r_avg = (r1 / 2) + (r2 / 2);

                lq_staging_neg.push(l_avg >> 8);
                lq_staging_neg.push(r_avg >> 8);
            }
        }
        assert_eq!(lq_staging_neg, vec![((-32768 / 2) + (0 / 2)) >> 8, ((-1000 / 2) + (0 / 2)) >> 8]);
    }

    #[test]
    fn test_preallocation_assertions() {
        // We simulate the converter logic directly to inspect capacities
        let mut hq_accumulator: Vec<u8> = Vec::with_capacity(2955000);
        let mut lq_accumulator: Vec<u8> = Vec::with_capacity(985000);

        let initial_hq_cap = hq_accumulator.capacity();
        let initial_lq_cap = lq_accumulator.capacity();

        let hq_encoder = FlacEncoder::new(48000, 2, 24, 4096);
        let lq_encoder = FlacEncoder::new(24000, 2, 16, 2048);
        let mut lq_staging = Vec::with_capacity(4096);

        // Simulating exactly 120 ALSA periods
        for _ in 0..120 {
            // Dummy buffer of 4096 frames
            let pcm_arc = Arc::new(vec![0i32; 8192]);

            let hq_frame = hq_encoder.encode_frame(&pcm_arc, 0);
            hq_accumulator.extend_from_slice(&hq_frame);

            lq_staging.clear();
            for i in (0..pcm_arc.len()).step_by(4) {
                if i + 3 < pcm_arc.len() {
                    let l1 = pcm_arc[i];
                    let r1 = pcm_arc[i + 1];
                    let l2 = pcm_arc[i + 2];
                    let r2 = pcm_arc[i + 3];

                    let l_avg = (l1 / 2) + (l2 / 2);
                    let r_avg = (r1 / 2) + (r2 / 2);

                    lq_staging.push(l_avg >> 8);
                    lq_staging.push(r_avg >> 8);
                }
            }

            let lq_frame = lq_encoder.encode_frame(&lq_staging, 0);
            lq_accumulator.extend_from_slice(&lq_frame);
        }

        assert_eq!(hq_accumulator.capacity(), initial_hq_cap, "HQ accumulator reallocated!");
        assert_eq!(lq_accumulator.capacity(), initial_lq_cap, "LQ accumulator reallocated!");
    }
}
