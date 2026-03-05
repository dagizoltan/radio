use crate::alsa_sys::*;
use libc::{EAGAIN, ENODEV, EPIPE, EWOULDBLOCK};
use std::os::unix::io::RawFd;
use tokio::io::unix::AsyncFd;

pub struct CaptureLoop {
    async_fd: AsyncFd<RawFd>,
    channels: u32,
}

impl CaptureLoop {
    pub fn new(fd: RawFd, channels: u32) -> std::io::Result<Self> {
        let async_fd = AsyncFd::new(fd)?;
        Ok(CaptureLoop { async_fd, channels })
    }

    pub async fn read_period(&self) -> std::io::Result<(Vec<i32>, bool)> {
        loop {
            let mut guard = self.async_fd.readable().await?;

            let samples_per_frame = self.channels as usize;
            let total_samples = 4096 * samples_per_frame;
            let mut raw_buffer = vec![0u32; total_samples];

            let mut xferi = SndrPcmXferi {
                result: 0,
                buf: raw_buffer.as_mut_ptr() as *mut i32,
                frames: 4096,
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

                // Sign-Extension: Iterate over raw u32 words, extract 24-bit audio
                // Always return stereo to the rest of the application
                let mut pcm_out = Vec::with_capacity(frames_read * 2);
                for frame in 0..frames_read {
                    let base = frame * samples_per_frame;
                    let l_raw = raw_buffer[base];
                    let r_raw = raw_buffer[base + 1]; // Guaranteed at least 2 channels are supported 

                    let l: i32 = (l_raw as i32) >> 8;
                    let r: i32 = (r_raw as i32) >> 8;
                    pcm_out.push(l);
                    pcm_out.push(r);
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
