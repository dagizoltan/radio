use crate::alsa_sys::*;
use libc::{EAGAIN, ENODEV, EPIPE, EWOULDBLOCK};
use std::os::unix::io::RawFd;
use tokio::io::unix::AsyncFd;

pub struct CaptureLoop {
    async_fd: AsyncFd<RawFd>,
    total_channels: u32,
    left_channel_idx: u32,
    right_channel_idx: u32,
}

impl CaptureLoop {
    pub fn new(fd: RawFd, total_channels: u32, left_channel_idx: u32, right_channel_idx: u32) -> std::io::Result<Self> {
        let async_fd = AsyncFd::new(fd)?;
        Ok(CaptureLoop {
            async_fd,
            total_channels,
            left_channel_idx,
            right_channel_idx,
        })
    }

    pub async fn read_period(&self) -> std::io::Result<(Vec<i32>, bool)> {
        loop {
            let mut guard = self.async_fd.readable().await?;

            // Try to read one period (4096 frames)
            let raw_buffer_len = 4096 * self.total_channels as usize;
            let mut raw_buffer = vec![0u32; raw_buffer_len];

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

                    // Synthesize a zero-padded buffer of exactly 4096 * 2 (8192) 0i32 values for our stereo output.
                    let silence = vec![0i32; 4096 * 2];

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

                // Sign-Extension & Downmix: Iterate over raw u32 words, extract 24-bit audio
                // for the specified left and right channels, discarding the rest.
                let mut pcm_out = Vec::with_capacity(frames_read * 2);
                let channels = self.total_channels as usize;
                let left_idx = self.left_channel_idx as usize;
                let right_idx = self.right_channel_idx as usize;

                for frame in 0..frames_read {
                    let base_idx = frame * channels;

                    let raw_left = raw_buffer[base_idx + left_idx];
                    let sample_left: i32 = (raw_left as i32) << 8 >> 8;
                    pcm_out.push(sample_left);

                    let raw_right = raw_buffer[base_idx + right_idx];
                    let sample_right: i32 = (raw_right as i32) << 8 >> 8;
                    pcm_out.push(sample_right);
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
        // Feed 0x00_80_00_00 (which is negative in 24-bit two's complement)
        let raw_word: u32 = 0x00800000;
        let sample: i32 = (raw_word as i32) << 8 >> 8;
        assert_eq!(sample, -8388608);
    }
}
