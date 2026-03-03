pub struct BitWriter {
    buffer: Vec<u8>,
    accumulator: u64,
    bits_in_accumulator: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter {
            buffer: Vec::new(),
            accumulator: 0,
            bits_in_accumulator: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        BitWriter {
            buffer: Vec::with_capacity(capacity),
            accumulator: 0,
            bits_in_accumulator: 0,
        }
    }

    pub fn write_bits(&mut self, val: u64, bits: u8) {
        if bits == 0 {
            return;
        }

        let mut bits = bits;
        let mask = if bits == 64 { u64::MAX } else { (1 << bits) - 1 };
        let mut val = val & mask;

        while bits > 0 {
            let space = 64 - self.bits_in_accumulator;
            let to_write = std::cmp::min(bits, space);

            let shift = bits - to_write;
            let chunk = val >> shift;

            let chunk_mask = if to_write == 64 { u64::MAX } else { (1 << to_write) - 1 };

            if to_write == 64 {
                self.accumulator = chunk & chunk_mask;
            } else {
                self.accumulator = (self.accumulator << to_write) | (chunk & chunk_mask);
            }
            self.bits_in_accumulator += to_write;

            bits -= to_write;

            if bits < 64 {
                val = val & ((1 << bits) - 1);
            }

            while self.bits_in_accumulator >= 8 {
                let shift = self.bits_in_accumulator - 8;
                let byte = (self.accumulator >> shift) as u8;
                self.buffer.push(byte);
                self.bits_in_accumulator -= 8;
            }

            if self.bits_in_accumulator > 0 {
                let mask = if self.bits_in_accumulator == 64 { u64::MAX } else { (1 << self.bits_in_accumulator) - 1 };
                self.accumulator = self.accumulator & mask;
            } else {
                self.accumulator = 0;
            }
        }
    }

    pub fn flush(&mut self) {
        if self.bits_in_accumulator > 0 {
            let padding = 8 - self.bits_in_accumulator;
            self.write_bits(0, padding);
        }
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        self.flush();
        self.buffer
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitwriter_bounds() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b101, 3);
        bw.write_bits(0b11111, 5);
        let bytes = bw.into_bytes();
        assert_eq!(bytes, vec![0xBF]);
    }

    #[test]
    fn test_cross_byte_boundaries() {
        let mut bw = BitWriter::new();
        // write 3 bits: 0b101
        bw.write_bits(0b101, 3);
        // write 24 bits: 0xAABBCC
        bw.write_bits(0xAABBCC, 24);
        bw.flush();
        // 101_10101010_10111011_11001100_00000
        // 10110101_01010111_01111001_10000000
        // 0xB5_57_79_80
        assert_eq!(bw.into_bytes(), vec![0xB5, 0x57, 0x79, 0x80]);
    }
}
