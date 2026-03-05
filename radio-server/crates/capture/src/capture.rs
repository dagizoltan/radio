use crate::alsa_sys::*;
use libc::{EAGAIN, ENODEV, EPIPE, EWOULDBLOCK};
use std::os::unix::io::RawFd;
use tokio::io::unix::AsyncFd;

pub struct CaptureLoop {
    async_fd: AsyncFd<RawFd>,
    channels: u32,
    format: u32,
    period_size: u32,
}

impl CaptureLoop {
    pub fn new(fd: RawFd, channels: u32, format: u32, period_size: u32) -> std::io::Result<Self> {
        let async_fd = AsyncFd::new(fd)?;
        Ok(CaptureLoop { async_fd, channels, format, period_size })
    }

    pub async fn read_period(&self) -> std::io::Result<(Vec<i32>, bool)> {
        loop {
            let mut guard = self.async_fd.readable().await?;

            let frames_to_read = self.period_size as usize;
            let samples_per_frame = self.channels as usize;
            let total_samples = frames_to_read * samples_per_frame;

            let _is_3byte = self.format == SNDRV_PCM_FORMAT_S24_3LE;
            let mut raw_buffer = vec![0u32; total_samples];

            let mut xferi = SndrPcmXferi {
                result: 0,
                buf: raw_buffer.as_mut_ptr() as *mut i32,
                frames: self.period_size as u64,
            };

            let ret = unsafe {
                libc::ioctl(
                    *self.async_fd.get_ref(),
                    SNDRV_PCM_IOCTL_READI_FRAMES as _,
                    &mut xferi,
                )
            };

            if ret < 0 {
                let err = std::io::Error::last_os_error();
                let errno = err.raw_os_error().unwrap_or(0);

                if errno == EPIPE {
                    // XRUN Recovery
                    eprintln!("WARN: ALSA buffer overrun (EPIPE)");

                    // Synthesize a zero-padded stereo buffer
                    let silence = vec![0i32; 8192];

                    // Call IOCTL_PREPARE to reset the hardware.
                    unsafe { libc::ioctl(*self.async_fd.get_ref(), SNDRV_PCM_IOCTL_PREPARE as _) };

                    // Return the silence buffer and indicate overrun
                    return Ok((silence, true));
                } else if errno == EAGAIN || errno == EWOULDBLOCK {
                    guard.clear_ready();
                    continue;
                } else if errno == ENODEV {
                    eprintln!("FATAL: Device disconnected mid-stream");
                    return Err(err);
                } else {
                    return Err(err);
                }
            } else {
                let frames_read = xferi.result as usize;
                if frames_read == 0 {
                    guard.clear_ready();
                    continue;
                }

                // Always return stereo to the rest of the application
                let mut pcm_out = Vec::with_capacity(frames_read * 2);

                if self.format == SNDRV_PCM_FORMAT_S24_3LE {
                    let raw_bytes = unsafe {
                        std::slice::from_raw_parts(
                            raw_buffer.as_ptr() as *const u8,
                            frames_read * samples_per_frame * 3
                        )
                    };
                    for frame in 0..frames_read {
                        let base = frame * samples_per_frame * 3;
                        let l_bytes = [raw_bytes[base], raw_bytes[base + 1], raw_bytes[base + 2]];
                        let r_bytes = [raw_bytes[base + 3], raw_bytes[base + 4], raw_bytes[base + 5]];

                        // Sign-extend from 24-bit to 32-bit (little-endian)
                        let l_raw = (l_bytes[0] as i32) | ((l_bytes[1] as i32) << 8) | ((l_bytes[2] as i8 as i32) << 16);
                        let r_raw = (r_bytes[0] as i32) | ((r_bytes[1] as i32) << 8) | ((r_bytes[2] as i8 as i32) << 16);

                        pcm_out.push(l_raw);
                        pcm_out.push(r_raw);
                    }
                } else if self.format == SNDRV_PCM_FORMAT_S16_LE {
                    let raw_i16 = unsafe {
                        std::slice::from_raw_parts(
                            raw_buffer.as_ptr() as *const i16,
                            frames_read * samples_per_frame
                        )
                    };
                    for frame in 0..frames_read {
                        let base = frame * samples_per_frame;
                        let l_raw = raw_i16[base] as i32;
                        let r_raw = raw_i16[base + 1] as i32;
                        // Scale 16-bit to 24-bit range
                        pcm_out.push(l_raw << 8);
                        pcm_out.push(r_raw << 8);
                    }
                } else if self.format == SNDRV_PCM_FORMAT_S24_LE {
                    for frame in 0..frames_read {
                        let base = frame * samples_per_frame;
                        let l_raw = raw_buffer[base];
                        let r_raw = raw_buffer[base + 1];

                        // sign extension for 24-bit in 32-bit
                        let l_ext = ((l_raw as i32) << 8) >> 8;
                        let r_ext = ((r_raw as i32) << 8) >> 8;

                        pcm_out.push(l_ext);
                        pcm_out.push(r_ext);
                    }
                } else {
                    // Default assume S32_LE format and shift 8
                    for frame in 0..frames_read {
                        let base = frame * samples_per_frame;
                        let l_raw = raw_buffer[base];
                        let r_raw = raw_buffer[base + 1];

                        let l: i32 = (l_raw as i32) >> 8;
                        let r: i32 = (r_raw as i32) >> 8;
                        pcm_out.push(l);
                        pcm_out.push(r);
                    }
                }

                return Ok((pcm_out, false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_sign_extension() {
        // Feed 0x80_00_00_00 (which is negative in 32-bit two's complement)
        let raw_word: u32 = 0x80000000;
        let sample: i32 = (raw_word as i32) >> 8;
        assert_eq!(sample, -8388608);
    }
}
