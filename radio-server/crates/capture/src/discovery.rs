use std::fs;

pub fn get_available_devices() -> Vec<(String, String)> {
    let mut devices = vec![("mock_device".to_string(), "Mock Device (Silence)".to_string())];
    
    if let Ok(cards) = fs::read_to_string("/proc/asound/cards") {
        for line in cards.lines() {
            // ALSA card lines look like: " 1 [UMC404HD       ]: USB-Audio - UMC404HD 192k"
            if line.starts_with(' ') && line.contains('[') && line.contains(']') {
                let parts: Vec<&str> = line.split('[').collect();
                if parts.len() == 2 {
                    let num_str = parts[0].trim();
                    let name_str = parts[1].split(']').next().unwrap_or("Unknown").trim();
                    let device_path = format!("/dev/snd/pcmC{}D0c", num_str);
                    let device_label = format!("ALSA Card {} - {}", num_str, name_str);
                    devices.push((device_path, device_label));
                }
            }
        }
    }
    
    devices
}
