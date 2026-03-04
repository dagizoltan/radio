use crate::alsa_sys::*;
use libc::{ioctl, O_NONBLOCK};
use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, RawFd};

pub struct Device {
    fd: RawFd,
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

        let mut hw_params = SndrPcmHwParams::default();

        let _mask_idx_access = SNDRV_PCM_HW_PARAM_ACCESS;
        let _mask_idx_format = SNDRV_PCM_HW_PARAM_FORMAT;

        hw_params.masks[0].bits[0] = 1 << SNDRV_PCM_ACCESS_RW_INTERLEAVED;

        let set_interval = |params: &mut SndrPcmHwParams, param_idx: usize, val: u32| {
            let idx = param_idx - 8;
            params.intervals[idx].min = val;
            params.intervals[idx].max = val;
            params.intervals[idx].flags = 2; // integer
        };

        // Attempt formats: S32_LE, S24_LE, S16_LE
        let formats_to_try = [
            SNDRV_PCM_FORMAT_S32_LE,
            SNDRV_PCM_FORMAT_S24_LE,
            SNDRV_PCM_FORMAT_S16_LE,
        ];

        let mut success = false;
        let mut actual_format = 0;

        for &fmt in &formats_to_try {
            hw_params.masks[1].bits[0] = 1 << fmt;
            hw_params.rmask = !0; // Request all params

            set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_RATE, 48000);
            set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_CHANNELS, 2);
            set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, 4096);
            set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_PERIODS, 4);

            let ret = unsafe { ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS as _, &mut hw_params) };
            if ret >= 0 {
                success = true;
                actual_format = fmt;
                break;
            }
        }

        if !success {
            return Err("Failed to set HW_PARAMS on device with any supported format".into());
        }

        // Validation: Verify the device didn't fallback to an unsupported rate or format constraint.
        if hw_params.masks[1].bits[0] & (1 << actual_format) == 0 {
            return Err("Device fallback: hw format negotiation failed".into());
        }
        if hw_params.intervals[SNDRV_PCM_HW_PARAM_RATE - 8].min != 48000 {
            return Err("Device fallback: does not support 48000 Hz".into());
        }

        Ok(Device { fd, _file: file })
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
}

// For custom_flags on OpenOptions
use std::os::unix::fs::OpenOptionsExt;
