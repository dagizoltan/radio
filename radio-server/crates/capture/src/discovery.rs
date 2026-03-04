use std::env;
use std::fs;

#[derive(Debug)]
pub struct AudioDevice {
    pub card_num: String,
    pub name: String,
}

pub fn list_devices() -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    if let Ok(cards) = fs::read_to_string("/proc/asound/cards") {
        for line in cards.lines() {
            // Lines typically look like " 1 [UMC404HD       ]: USB-Audio - UMC404HD 192k"
            // or " 0 [PCH            ]: HDA-Intel - HDA Intel PCH"
            // Wait, there are usually two lines per device, the first one starts with a number.
            if line.trim().starts_with(|c: char| c.is_ascii_digit()) {
                let parts: Vec<&str> = line.trim().splitn(2, '[').collect();
                if parts.len() == 2 {
                    let card_num = parts[0].trim().to_string();
                    let name_parts: Vec<&str> = parts[1].splitn(2, ']').collect();
                    if name_parts.len() == 2 {
                        let name = name_parts[0].trim().to_string();
                        devices.push(AudioDevice { card_num, name });
                    }
                }
            }
        }
    }
    devices
}

pub fn discover_device() -> (String, u32) {
    let devices = list_devices();

    // Determine the target device name from the environment or default to UMC404HD
    let target_device_name =
        env::var("AUDIO_DEVICE").unwrap_or_else(|_| "UMC404HD".to_string());

    for device in &devices {
        if device.name.contains(&target_device_name) {
            let channels = env::var("AUDIO_CHANNELS").unwrap_or_else(|_| "4".to_string());
            let channels: u32 = channels.parse().unwrap_or(4);
            return (format!("/dev/snd/pcmC{}D0c", device.card_num), channels);
        }
    }

    // Fallback: If AUDIO_DEVICE wasn't explicitly set, and UMC404HD isn't found,
    // try to find the internal ThinkPad X270 microphone (usually "PCH" or similar Intel audio)
    if env::var("AUDIO_DEVICE").is_err() {
        for device in &devices {
            if device.name.contains("PCH") || device.name.contains("Intel") {
                println!("Fallback to integrated audio device ({})", device.name);
                // Integrated microphones are typically 2 channels
                return (format!("/dev/snd/pcmC{}D0c", device.card_num), 2);
            }
        }
    }

    // Ultimate fallback for environments without real ALSA devices (e.g., Docker/macOS mock)
    if devices.is_empty() || env::var("AUDIO_DEVICE").unwrap_or_default() == "mock_device" {
        println!("No ALSA devices found or mock requested. Falling back to mock_device.");
        return ("mock_device".to_string(), 2);
    }

    panic!("Device {} not found. Available devices: {:?}", target_device_name, devices);
}
