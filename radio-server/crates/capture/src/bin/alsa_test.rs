use alsa::pcm::{PCM, HwParams, Format, Access};
use alsa::Direction;

fn main() {
    let dev_name = "hw:1,0"; // Card 1, Device 0
    println!("Opening {}", dev_name);
    match PCM::new(dev_name, Direction::Capture, false) {
        Ok(pcm) => {
            match HwParams::any(&pcm) {
                Ok(hwp) => {
                    println!("Min Rate: {:?}", hwp.get_rate_min());
                    println!("Max Rate: {:?}", hwp.get_rate_max());
                    println!("Min Channels: {:?}", hwp.get_channels_min());
                    println!("Max Channels: {:?}", hwp.get_channels_max());
                    
                    for fmt in [Format::S16LE, Format::S24LE, Format::S243LE, Format::S32LE] {
                        let ok = hwp.test_format(fmt).is_ok();
                        println!("Supports {:?}: {}", fmt, ok);
                    }
                },
                Err(e) => println!("HwParams::any error: {}", e),
            }
        },
        Err(e) => println!("PCM::new error: {}", e),
    }
}
