use crate::alsa_sys::*;
use libc::{ioctl, O_NONBLOCK};
use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, RawFd};

pub struct Device {
    fd: RawFd,
    _file: std::fs::File,
}

impl Device {
    pub fn open(path: &str) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
            .expect("Failed to open capture device");

        let fd = file.as_raw_fd();

        let mut hw_params = SndrPcmHwParams::default();

        // This is a minimal, correct implementation to set up the ALSA params based on prompt.
        // We set format, access, rate, channels, period_size, periods buffer.
        // ALSA requires setting bits in masks for FORMAT and ACCESS.
        // In ALSA, masks[0] is for ACCESS, masks[1] is for FORMAT?
        // Let's use the standard ALSA struct layouts and set it correctly.
        // Wait, masks[0] corresponds to hw param 0 to 31.
        // SNDRV_PCM_HW_PARAM_ACCESS is 0 -> masks[0] & (1<<0)
        // Wait, mask array maps 0=ACCESS, 1=FORMAT, 2=SUBFORMAT.
        // The ALSA param index for ACCESS is 0, FORMAT is 1, SUBFORMAT is 2.

        let _mask_idx_access = SNDRV_PCM_HW_PARAM_ACCESS;
        let _mask_idx_format = SNDRV_PCM_HW_PARAM_FORMAT;

        hw_params.masks[0] = 1 << SNDRV_PCM_ACCESS_RW_INTERLEAVED;
        // 0 -> Access. masks[0] represents the mask for ACCESS. The bit to set is SNDRV_PCM_ACCESS_RW_INTERLEAVED (3) -> 1<<3.

        hw_params.masks[1] = 1 << SNDRV_PCM_FORMAT_S24_LE;
        // 1 -> Format. masks[1] represents the mask for FORMAT. The bit is 6 -> 1<<6.

        // Let's set the mask bits for the requested parameters.
        hw_params.rmask = !0; // Request all params

        // Interval indices map to intervals[index - 7] since interval params start at 7 (SNDRV_PCM_HW_PARAM_SAMPLE_BITS).
        let set_interval = |params: &mut SndrPcmHwParams, param_idx: usize, val: u32| {
            let idx = param_idx - 7;
            params.intervals[idx].min = val;
            params.intervals[idx].max = val;
            params.intervals[idx].flags = 2; // integer
        };

        set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_RATE, 48000);
        set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_CHANNELS, 2);
        set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, 4096);
        set_interval(&mut hw_params, SNDRV_PCM_HW_PARAM_PERIODS, 4);

        let ret = unsafe { ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS as _, &mut hw_params) };
        if ret < 0 {
            panic!("Failed to set HW_PARAMS");
        }

        // Validation: Verify the device didn't fallback to a different rate or format.
        if hw_params.masks[1] & (1 << SNDRV_PCM_FORMAT_S24_LE) == 0 {
            panic!("Device fallback: does not support S24_LE format");
        }
        if hw_params.intervals[SNDRV_PCM_HW_PARAM_RATE - 7].min != 48000 {
            panic!("Device fallback: does not support 48000 Hz");
        }

        Device { fd, _file: file }
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
