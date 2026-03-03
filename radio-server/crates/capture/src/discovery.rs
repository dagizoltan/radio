use std::fs;

const CAPTURE_DEVICE_NAME: &str = "UMC404HD";

pub fn discover_device() -> String {
    let cards =
        fs::read_to_string("/proc/asound/cards").expect("Failed to read /proc/asound/cards");

    for line in cards.lines() {
        if line.contains(CAPTURE_DEVICE_NAME) {
            // " 1 [UMC404HD       ]: USB-Audio - UMC404HD 192k"
            let card_num = line.trim().split_whitespace().next().unwrap();
            return format!("/dev/snd/pcmC{}D0c", card_num);
        }
    }

    panic!("Device {} not found", CAPTURE_DEVICE_NAME);
}
