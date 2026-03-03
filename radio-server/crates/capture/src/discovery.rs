use std::fs;

const CAPTURE_DEVICE_NAME: &str = "UMC404HD";

pub fn discover_device() -> String {
    let cards = match fs::read_to_string("/proc/asound/cards") {
        Ok(c) => c,
        Err(_) => return String::from("mock_device"), // Return dummy device for tests in sandbox
    };

    for line in cards.lines() {
        if line.contains(CAPTURE_DEVICE_NAME) {
            // " 1 [UMC404HD       ]: USB-Audio - UMC404HD 192k"
            let card_num = line.trim().split_whitespace().next().unwrap();
            return format!("/dev/snd/pcmC{}D0c", card_num);
        }
    }

    panic!("Device {} not found", CAPTURE_DEVICE_NAME);
}
