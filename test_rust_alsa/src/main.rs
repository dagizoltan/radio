use alsa::pcm::*;
use std::ffi::CString;
fn main() {
    let pcm = PCM::new("hw:1,0", Direction::Capture, false).unwrap();
    {
        let hwp = HwParams::any(&pcm).unwrap();
        println!("Supported Rates: Min {}, Max {}", hwp.get_rate_min().unwrap(), hwp.get_rate_max().unwrap());
        println!("Supported Channels: Min {}, Max {}", hwp.get_channels_min().unwrap(), hwp.get_channels_max().unwrap());
        
        let fmt = pcm.format_name(Format::S32LE).unwrap_or("unknown".to_string());
        println!("Supports S32_LE format: {}", hwp.test_format(Format::S32LE).is_ok());
        println!("Supports S24_3LE format: {}", hwp.test_format(Format::S243LE).is_ok());
        println!("Supports S24_LE format: {}", hwp.test_format(Format::S24LE).is_ok());
    }
}
