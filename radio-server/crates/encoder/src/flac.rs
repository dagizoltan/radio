use crate::bitwriter::BitWriter;
use crate::crc::{Crc8, Crc16};

pub struct FlacEncoder {
    sample_rate: u32,
    channels: u32,
    bps: u32,
    block_size: u32,
    crc8: Crc8,
    crc16: Crc16,
}

impl FlacEncoder {
    pub fn new(sample_rate: u32, channels: u32, bps: u32, block_size: u32) -> Self {
        FlacEncoder {
            sample_rate,
            channels,
            bps,
            block_size,
            crc8: Crc8::new(),
            crc16: Crc16::new(),
        }
    }

    pub fn stream_header(&self) -> Vec<u8> {
        let mut bw = BitWriter::new();
        // fLaC
        bw.write_bits(0x664C6143, 32);

        // 0x80 block header byte + 24-bit length 0x000022
        // Wait, length is 34 bytes (0x22). 0x80 means last metadata block, type 0 (STREAMINFO) -> 0x80.
        bw.write_bits(0x80000022, 32);

        // Bit-packed fields:
        // 16-bit min block, 16-bit max block
        bw.write_bits(self.block_size as u64, 16);
        bw.write_bits(self.block_size as u64, 16);
        // 24-bit min frame, 24-bit max frame
        bw.write_bits(0, 24);
        bw.write_bits(0, 24);
        // 20-bit sample rate, 3-bit channels (channels-1), 5-bit bps (bps-1), 36-bit total samples
        bw.write_bits(self.sample_rate as u64, 20);
        bw.write_bits((self.channels - 1) as u64, 3);
        bw.write_bits((self.bps - 1) as u64, 5);
        bw.write_bits(0, 36);
        // 128-bit MD5
        bw.write_bits(0, 64);
        bw.write_bits(0, 64);

        bw.into_bytes()
    }

    pub fn encode_frame(&self, interleaved: &[i32], frame_number: u64) -> Vec<u8> {
        let mut bw = BitWriter::with_capacity(16 + interleaved.len() * 4);

        // Frame header
        bw.write_bits(0x3FFE, 14); // Sync code
        bw.write_bits(0, 1); // Reserved
        bw.write_bits(0, 1); // Blocking strategy (0 = fixed block size)

        bw.write_bits(0b0111, 4); // Block size: 0111 (get from end of header)
        bw.write_bits(0b1100, 4); // Sample rate: 1100 (get from end of header)

        // Channel assignment
        let channel_assign = if self.channels == 2 { 0b0001 } else { 0b0000 };
        bw.write_bits(channel_assign, 4);

        // Sample size (bps)
        // Fixed codes for bps: 0b110 for 24-bit, but I should maybe support 16-bit?
        // Let's use 0b110 for 24-bit and 0b100 for 16-bit. Prompt says: "Write fixed codes ... bps (0b110)"
        let bps_code = if self.bps == 24 { 0b110 } else { 0b100 };
        bw.write_bits(bps_code, 3);
        bw.write_bits(0, 1); // Reserved

        // UTF-8 encoded frame_number
        let encoded_frame_number = Self::encode_utf8_flac(frame_number);
        for byte in encoded_frame_number {
            bw.write_bits(byte as u64, 8);
        }

        // literal block size (block_size - 1) as 16-bit because of 0b0111 code
        bw.write_bits((self.block_size - 1) as u64, 16);

        // literal rate as 16-bit because of 0b1100 code
        bw.write_bits(self.sample_rate as u64, 16);

        // CRC-8 of header
        let header_bytes = bw.bytes();
        let header_crc = self.crc8.calculate(header_bytes);
        bw.write_bits(header_crc as u64, 8);

        // Subframes
        for channel in 0..self.channels as usize {
            // Subframe header: 0b00000010 (verbatim, no wasted bits)
            bw.write_bits(0b00000010, 8);

            // Raw samples byte-aligned.
            for i in 0..self.block_size as usize {
                // interleaved: L R L R
                let sample = interleaved[i * self.channels as usize + channel];

                // For 24-bit we write 24 bits
                if self.bps == 24 {
                    bw.write_bits((sample & 0xFFFFFF) as u64, 24);
                } else if self.bps == 16 {
                    bw.write_bits((sample & 0xFFFF) as u64, 16);
                }
            }
        }

        bw.flush();

        // CRC-16 of entire frame
        let frame_bytes = bw.bytes();
        let frame_crc = self.crc16.calculate(frame_bytes);
        bw.write_bits(frame_crc as u64, 16);

        bw.into_bytes()
    }

    fn encode_utf8_flac(val: u64) -> Vec<u8> {
        if val < 0x80 {
            vec![val as u8]
        } else if val < 0x800 {
            vec![(0xC0 | (val >> 6)) as u8, (0x80 | (val & 0x3F)) as u8]
        } else if val < 0x10000 {
            vec![
                (0xE0 | (val >> 12)) as u8,
                (0x80 | ((val >> 6) & 0x3F)) as u8,
                (0x80 | (val & 0x3F)) as u8,
            ]
        } else if val < 0x200000 {
            vec![
                (0xF0 | (val >> 18)) as u8,
                (0x80 | ((val >> 12) & 0x3F)) as u8,
                (0x80 | ((val >> 6) & 0x3F)) as u8,
                (0x80 | (val & 0x3F)) as u8,
            ]
        } else if val < 0x4000000 {
            vec![
                (0xF8 | (val >> 24)) as u8,
                (0x80 | ((val >> 18) & 0x3F)) as u8,
                (0x80 | ((val >> 12) & 0x3F)) as u8,
                (0x80 | ((val >> 6) & 0x3F)) as u8,
                (0x80 | (val & 0x3F)) as u8,
            ]
        } else {
            vec![
                (0xFC | (val >> 30)) as u8,
                (0x80 | ((val >> 24) & 0x3F)) as u8,
                (0x80 | ((val >> 18) & 0x3F)) as u8,
                (0x80 | ((val >> 12) & 0x3F)) as u8,
                (0x80 | ((val >> 6) & 0x3F)) as u8,
                (0x80 | (val & 0x3F)) as u8,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::crc::{Crc8, Crc16};

    #[test]
    fn test_flac_crc_vectors() {
        // We will just verify the CRC8 and CRC16 logic on a known buffer
        let crc8 = Crc8::new();
        let crc16 = Crc16::new();
        let data = b"123456789";
        let c8 = crc8.calculate(data);
        let c16 = crc16.calculate(data);
        assert_eq!(c8, 0xF4);
        assert_eq!(c16, 0xFEE8);
    }
}
