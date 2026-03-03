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

        // Mask out any bits above 'bits' just in case
        let mask = if bits == 64 {
            u64::MAX
        } else {
            (1 << bits) - 1
        };
        let val = val & mask;

        self.accumulator = (self.accumulator << bits) | val;
        self.bits_in_accumulator += bits;

        while self.bits_in_accumulator >= 8 {
            self.bits_in_accumulator -= 8;
            let byte = (self.accumulator >> self.bits_in_accumulator) as u8;
            self.buffer.push(byte);
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
        bw.write_bits(0b110, 3);
        bw.write_bits(0b11111, 5);
        let bytes = bw.into_bytes();
        assert_eq!(bytes, vec![0xDF]);
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
