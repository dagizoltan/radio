use crate::alsa_sys::*;
use libc::{ioctl, O_NONBLOCK};
use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, RawFd};

pub struct Device {
    fd: RawFd,
    channels: u32,
    actual_format: u32,
    period_size: u32,
    _file: std::fs::File,
}

impl Device {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("Failed to open capture device {}: {}", path, e))?;

        let fd = file.as_raw_fd();

        let set_interval = |params: &mut SndrPcmHwParams, param_idx: usize, min_val: u32, max_val: u32| {
            let idx = param_idx - 8;
            params.intervals[idx].min = min_val;
            params.intervals[idx].max = max_val;
            params.intervals[idx].flags = 0; // inclusive range
        };

        // Attempt formats: S32_LE, S24_3LE, S24_LE, S16_LE
        let formats_to_try = [
            SNDRV_PCM_FORMAT_S32_LE,
            SNDRV_PCM_FORMAT_S24_3LE,
            SNDRV_PCM_FORMAT_S24_LE,
            SNDRV_PCM_FORMAT_S16_LE,
        ];

        let mut success = false;
        let mut actual_format = 0;
        let mut actual_channels = 0;
        let mut actual_period_size = 0;
        let mut final_hw_params = SndrPcmHwParams::default();

        for &fmt in &formats_to_try {
            for &ch in &[4, 2, 8, 1] { // Prioritize 4 channels for UMC404HD, fallback to 2, 8, 1
                // Behringer devices often demand very specific period sizes based on sample rate.
                // We'll also try a wildcard buffer sizing if explicit period sizes fail.
                for &period_size in &[4096, 2048, 1024, 512, 256, 128] {
                    let mut hw_params = SndrPcmHwParams::default();

                    // 1. Constrain ACCESS (Interleaved)
                    hw_params.masks[0].bits[0] = 1 << SNDRV_PCM_ACCESS_RW_INTERLEAVED;

                    // 2. Constrain FORMAT
                    let fmt_idx = (fmt / 32) as usize;
                    let fmt_bit = fmt % 32;
                    hw_params.masks[1].bits[fmt_idx] = 1 << fmt_bit;

                    // 3. Constrain SUBFORMAT (Standard)
                    hw_params.masks[2].bits[0] = 1 << 0; // SUBFORMAT_STD

                    // 4. Set rmask for masks
                    hw_params.rmask |= (1 << SNDRV_PCM_HW_PARAM_ACCESS) |
                                     (1 << SNDRV_PCM_HW_PARAM_FORMAT) |
                                     (1 << SNDRV_PCM_HW_PARAM_SUBFORMAT);

                    // Initialize all intervals to [0, u32::MAX]
                    for iv in hw_params.intervals.iter_mut() {
                        iv.min = 0; iv.max = u32::MAX;
                        iv.flags = 0;
                    }

                    // Strict intervals (min == max)
                    set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_RATE, 48000, 48000);
                    set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_CHANNELS, ch, ch);
                    set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, period_size, period_size);

                    // Allow hardware to determine the best buffer size and periods by leaving their intervals open
                    // Set rmask for intervals to strictly enforce RATE, CHANNELS, and PERIOD_SIZE
                    hw_params.rmask |= (1 << SNDRV_PCM_HW_PARAM_RATE) |
                                     (1 << SNDRV_PCM_HW_PARAM_CHANNELS) |
                                     (1 << SNDRV_PCM_HW_PARAM_PERIOD_SIZE);

                    let ret = unsafe { ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS as _, &mut hw_params) };
                    if ret >= 0 {
                        success = true;
                        actual_format = fmt;
                        actual_channels = ch;
                        actual_period_size = period_size;
                        final_hw_params = hw_params;
                        println!("SUCCESS: Set HW_PARAMS (format={}, channels={}, period_size={})", fmt, ch, period_size);
                        break;
                    } else {
                        let err = std::io::Error::last_os_error();
                        println!("DEBUG: Failed HW_PARAMS (format={}, channels={}, period_size={}) -> err: {}", fmt, ch, period_size, err);
                    }
                }
                if success { break; }
            }
            if success { break; }
        }

        if !success {
            return Err("Failed to set HW_PARAMS on device with any supported format".into());
        }

        // Validation: Verify the device didn't fallback to an unsupported rate or format constraint.
        let actual_fmt_idx = (actual_format / 32) as usize;
        let actual_fmt_bit = actual_format % 32;
        if final_hw_params.masks[1].bits[actual_fmt_idx] & (1 << actual_fmt_bit) == 0 {
            return Err("Device fallback: hw format negotiation failed".into());
        }
        if final_hw_params.intervals[SNDRV_PCM_HW_PARAM_RATE - 8].min != 48000 {
            return Err("Device fallback: does not support 48000 Hz".into());
        }

        Ok(Device { fd, channels: actual_channels, actual_format, period_size: actual_period_size, _file: file })
    }

    pub fn prepare(&self) {
        let ret = unsafe { ioctl(self.fd, SNDRV_PCM_IOCTL_PREPARE as _) };
        if ret < 0 {
            eprintln!("Failed to PREPARE device");
        }
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    pub fn format(&self) -> u32 {
        self.actual_format
    }

    pub fn period_size(&self) -> u32 {
        self.period_size
    }
}

// For custom_flags on OpenOptions
use std::os::unix::fs::OpenOptionsExt;
