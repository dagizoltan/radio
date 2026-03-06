pub struct Crc8 {
    table: [u8; 256],
}

impl Default for Crc8 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc8 {
    pub fn new() -> Self {
        let mut table = [0u8; 256];
        for (i, entry) in table.iter_mut().enumerate().take(256) {
            let mut crc = i as u8;
            for _ in 0..8 {
                if crc & 0x80 != 0 {
                    crc = (crc << 1) ^ 0x07;
                } else {
                    crc <<= 1;
                }
            }
            *entry = crc;
        }
        Crc8 { table }
    }

    pub fn calculate(&self, data: &[u8]) -> u8 {
        let mut crc = 0u8;
        for &byte in data {
            crc = self.table[(crc ^ byte) as usize];
        }
        crc
    }
}

pub struct Crc16 {
    table: [u16; 256],
}

impl Default for Crc16 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc16 {
    pub fn new() -> Self {
        let mut table = [0u16; 256];
        for (i, entry) in table.iter_mut().enumerate().take(256) {
            let mut crc = (i as u16) << 8;
            for _ in 0..8 {
                if crc & 0x8000 != 0 {
                    crc = (crc << 1) ^ 0x8005;
                } else {
                    crc <<= 1;
                }
            }
            *entry = crc;
        }
        Crc16 { table }
    }

    pub fn calculate(&self, data: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &byte in data {
            let idx = ((crc >> 8) as u8 ^ byte) as usize;
            crc = (crc << 8) ^ self.table[idx];
        }
        crc
    }
}
