use crate::alsa_sys::*;
use libc::{EAGAIN, ENODEV, EPIPE, EWOULDBLOCK};
use std::io::Error;
use std::os::unix::io::RawFd;
use tokio::io::unix::AsyncFd;

pub struct CaptureLoop {
    async_fd: AsyncFd<RawFd>,
}

impl CaptureLoop {
    pub fn new(fd: RawFd) -> std::io::Result<Self> {
        let async_fd = AsyncFd::new(fd)?;
        Ok(CaptureLoop { async_fd })
    }

    pub async fn read_period(&self) -> std::io::Result<Vec<i32>> {
        loop {
            let mut guard = self.async_fd.readable().await?;

            // Try to read one period (4096 frames = 8192 samples)
            let mut raw_buffer = vec![0u32; 8192];

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
                    // radio_capture_overruns_total++ -> this would go in metrics.

                    // Synthesize a zero-padded buffer of exactly 4096 * 2 (8192) 0i32 values.
                    let silence = vec![0i32; 8192];

                    // Call IOCTL_PREPARE to reset the hardware.
                    unsafe { libc::ioctl(*self.async_fd.get_ref(), SNDRV_PCM_IOCTL_PREPARE as _) };

                    // Return the silence buffer
                    return Ok(silence);
                } else if errno == EAGAIN || errno == EWOULDBLOCK {
                    guard.clear_ready();
                    continue;
                } else if errno == ENODEV {
                    panic!("FATAL: Device disconnected mid-stream");
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
                let mut pcm_out = Vec::with_capacity(frames_read * 2);
                for i in 0..(frames_read * 2) {
                    let raw_word = raw_buffer[i];
                    let sample: i32 = (raw_word as i32) << 8 >> 8;
                    pcm_out.push(sample);
                }

                return Ok(pcm_out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_extension() {
        // Feed 0x00_80_00_00 (which is negative in 24-bit two's complement)
        let raw_word: u32 = 0x00800000;
        let sample: i32 = (raw_word as i32) << 8 >> 8;
        assert_eq!(sample, -8388608);
    }
}
