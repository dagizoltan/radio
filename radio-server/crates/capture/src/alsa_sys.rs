// alsa_sys.rs
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SndInterval {
    pub min: u32,
    pub max: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SndrPcmHwParams {
    pub flags: u32,
    pub masks: [u32; 16],
    pub mres: [u32; 20],
    pub intervals: [SndInterval; 12],
    pub ires: [u32; 36],
    pub rmask: u32,
    pub cmask: u32,
    pub info: u32,
    pub msbits: u32,
    pub rate_num: u32,
    pub rate_den: u32,
    pub fifo_size: u64,
    pub reserved: [u8; 64],
}

impl Default for SndrPcmHwParams {
    fn default() -> Self {
        SndrPcmHwParams {
            flags: 0,
            masks: [0; 16],
            mres: [0; 20],
            intervals: [SndInterval::default(); 12],
            ires: [0; 36],
            rmask: 0,
            cmask: 0,
            info: 0,
            msbits: 0,
            rate_num: 0,
            rate_den: 0,
            fifo_size: 0,
            reserved: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SndrPcmXferi {
    pub result: i64,
    pub buf: *mut i32,
    pub frames: u64,
}

// ioctl constants
pub const SNDRV_PCM_IOCTL_HW_PARAMS: usize = 0xc2604111;
pub const SNDRV_PCM_IOCTL_PREPARE: usize = 0x4140;
pub const SNDRV_PCM_IOCTL_READI_FRAMES: usize = 0x80184151;

// Mask constants
pub const SNDRV_PCM_HW_PARAM_ACCESS: usize = 0;
pub const SNDRV_PCM_HW_PARAM_FORMAT: usize = 1;
pub const SNDRV_PCM_HW_PARAM_SUBFORMAT: usize = 2;

// Interval constants
pub const SNDRV_PCM_HW_PARAM_SAMPLE_BITS: usize = 7;
pub const SNDRV_PCM_HW_PARAM_FRAME_BITS: usize = 8;
pub const SNDRV_PCM_HW_PARAM_CHANNELS: usize = 9;
pub const SNDRV_PCM_HW_PARAM_RATE: usize = 10;
pub const SNDRV_PCM_HW_PARAM_PERIOD_TIME: usize = 11;
pub const SNDRV_PCM_HW_PARAM_PERIOD_SIZE: usize = 12;
pub const SNDRV_PCM_HW_PARAM_PERIOD_BYTES: usize = 13;
pub const SNDRV_PCM_HW_PARAM_PERIODS: usize = 14;
pub const SNDRV_PCM_HW_PARAM_BUFFER_TIME: usize = 15;
pub const SNDRV_PCM_HW_PARAM_BUFFER_SIZE: usize = 16;
pub const SNDRV_PCM_HW_PARAM_BUFFER_BYTES: usize = 17;
pub const SNDRV_PCM_HW_PARAM_TICK_TIME: usize = 18;

// Specific formats and access
pub const SNDRV_PCM_FORMAT_S24_LE: u32 = 6;
pub const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
