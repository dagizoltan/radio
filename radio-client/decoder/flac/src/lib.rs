use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct FlacDecoder {
    accumulator: Vec<u8>,
    header_parsed: bool,
    sample_rate: u32,
    bps: u32,
    channels: u32,
    out_buffer: Vec<f32>,
}

#[wasm_bindgen]
impl FlacDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        FlacDecoder {
            accumulator: Vec::new(),
            header_parsed: false,
            sample_rate: 48000,
            bps: 24,
            channels: 2,
            out_buffer: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.accumulator.clear();
        self.header_parsed = false;
        self.out_buffer.clear();
    }

    pub fn push(&mut self, bytes: &[u8]) -> *const f32 {
        self.out_buffer.clear();
        self.accumulator.extend_from_slice(bytes);

        // Parse STREAMINFO header if not parsed
        if !self.header_parsed {
            if self.accumulator.len() < 42 {
                return self.out_buffer.as_ptr();
            }

            if &self.accumulator[0..4] != b"fLaC" {
                self.reset();
                return self.out_buffer.as_ptr();
            }

            let rate_chan_bps_samples = &self.accumulator[18..26];
            let rate = ((rate_chan_bps_samples[0] as u32) << 12)
                | ((rate_chan_bps_samples[1] as u32) << 4)
                | ((rate_chan_bps_samples[2] as u32) >> 4);
            let channels = ((rate_chan_bps_samples[2] >> 1) & 0x07) as u32 + 1;
            let bps = (((rate_chan_bps_samples[2] & 0x01) << 4) | (rate_chan_bps_samples[3] >> 4)) as u32 + 1;

            self.sample_rate = rate;
            self.channels = channels;
            self.bps = bps;
            self.header_parsed = true;

            self.accumulator.drain(0..42);
        }

        while self.accumulator.len() >= 2 {
            // Find sync code
            if self.accumulator[0] == 0xFF && (self.accumulator[1] & 0xFE) == 0xF8 {
                let start = 0; // The frame starts at index 0 of the accumulator

                // Tentative parse
                let mut pos = start + 2;

                if pos >= self.accumulator.len() { break; }
                let bs_sr = self.accumulator[pos]; pos += 1;

                if pos >= self.accumulator.len() { break; }
                let _ch_bps = self.accumulator[pos]; pos += 1;

                // UTF-8 frame number decoding
                if pos >= self.accumulator.len() { break; }
                let utf8_lead = self.accumulator[pos]; pos += 1;
                let mut utf8_bytes = 0;
                if utf8_lead < 0x80 { utf8_bytes = 0; }
                else if (utf8_lead & 0xE0) == 0xC0 { utf8_bytes = 1; }
                else if (utf8_lead & 0xF0) == 0xE0 { utf8_bytes = 2; }
                else if (utf8_lead & 0xF8) == 0xF0 { utf8_bytes = 3; }
                else if (utf8_lead & 0xFC) == 0xF8 { utf8_bytes = 4; }
                else if (utf8_lead & 0xFE) == 0xFC { utf8_bytes = 5; }
                else if (utf8_lead & 0xFF) == 0xFE { utf8_bytes = 6; }

                if pos + utf8_bytes > self.accumulator.len() { break; }
                pos += utf8_bytes;

                // Block size literal (if 0b0111 or 0b0110)
                let bs_code = bs_sr >> 4;
                let mut block_size = 0;
                if bs_code == 0b0111 {
                    if pos + 2 > self.accumulator.len() { break; }
                    block_size = ((self.accumulator[pos] as u32) << 8 | self.accumulator[pos+1] as u32) + 1;
                    pos += 2;
                } else if bs_code == 0b0110 {
                    if pos + 1 > self.accumulator.len() { break; }
                    block_size = (self.accumulator[pos] as u32) + 1;
                    pos += 1;
                }

                // Sample rate literal (if 0b1100)
                let sr_code = bs_sr & 0x0F;
                if sr_code == 0b1100 {
                    if pos + 2 > self.accumulator.len() { break; }
                    pos += 2;
                }

                // CRC-8
                if pos + 1 > self.accumulator.len() { break; }
                pos += 1;

                if block_size == 0 {
                    // Invalid/unsupported frame. Skip 1 byte to find next sync.
                    self.accumulator.remove(0);
                    continue;
                }

                // Calculate required bytes for verbatim subframes
                let bytes_per_sample = if self.bps == 24 { 3 } else if self.bps == 16 { 2 } else { 0 };
                let frame_data_bytes = self.channels as usize * (1 + block_size as usize * bytes_per_sample);

                if pos + frame_data_bytes + 2 > self.accumulator.len() {
                    break; // +2 for CRC-16
                }

                // We have the full frame! Decode it.
                for _ in 0..self.channels {
                    let sub_header = self.accumulator[pos]; pos += 1;
                    if sub_header != 0x02 {
                        // Not verbatim. Skip frame.
                        break;
                    }

                    for _ in 0..block_size {
                        let sample: i32;
                        if self.bps == 24 {
                            let raw_value = ((self.accumulator[pos] as u32) << 16)
                                | ((self.accumulator[pos+1] as u32) << 8)
                                | (self.accumulator[pos+2] as u32);
                            pos += 3;
                            sample = (raw_value as i32) << (32 - 24) >> (32 - 24);
                        } else if self.bps == 16 {
                            let raw_value = ((self.accumulator[pos] as u32) << 8)
                                | (self.accumulator[pos+1] as u32);
                            pos += 2;
                            sample = (raw_value as i32) << (32 - 16) >> (32 - 16);
                        } else {
                            sample = 0;
                        }

                        // Normalize
                        let normalized = sample as f32 / (1 << (self.bps - 1)) as f32;
                        self.out_buffer.push(normalized);
                    }
                }

                // CRC-16
                pos += 2;

                // Done parsing frame, remove from accumulator
                self.accumulator.drain(0..pos);
            } else {
                // Not sync code, scan forward
                self.accumulator.remove(0);
            }
        }

        self.out_buffer.as_ptr()
    }

    pub fn len(&self) -> usize {
        self.out_buffer.len()
    }
}
