use std::fs;
use std::collections::HashMap;

/// Scan /dev/snd/ for capture PCM nodes (pcmC*D*c) and attempt to resolve
/// human-readable card names from /proc/asound/cards when available.
pub fn get_available_devices() -> Vec<(String, String)> {
    let mut devices = vec![("mock_device".to_string(), "Mock Device (Silence)".to_string())];

    // Build a card-number → card-name map from /proc/asound/cards if accessible.
    // This file is available on the host but not inside Docker containers.
    let card_names = parse_asound_cards();

    // Scan /dev/snd/ for capture device nodes: pcmC{card}D{dev}c
    let snd_dir = "/dev/snd";
    let Ok(entries) = fs::read_dir(snd_dir) else { return devices; };

    let mut found: Vec<(String, String)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            // Match pcmC<card>D<dev>c  (capture nodes end with 'c')
            if !name.starts_with("pcmC") || !name.ends_with('c') {
                return None;
            }
            let path = format!("{}/{}", snd_dir, name);

            // Parse card number from e.g. "pcmC1D0c"
            let inner = name.strip_prefix("pcmC").unwrap_or(&name);
            let card_str = inner.split('D').next().unwrap_or("?");
            let card_num: u32 = card_str.parse().ok()?;
            let dev_str = inner
                .strip_prefix(card_str)
                .and_then(|s| s.strip_prefix('D'))
                .and_then(|s| s.strip_suffix('c'))
                .unwrap_or("?");

            let label = if let Some(card_name) = card_names.get(&card_num) {
                format!("Card {} ({}) — Device {}", card_num, card_name, dev_str)
            } else {
                format!("Card {} — Device {} (PCM Capture)", card_num, dev_str)
            };

            Some((path, label))
        })
        .collect();

    // Stable sort by path so the list is deterministic
    found.sort_by(|a, b| a.0.cmp(&b.0));
    devices.extend(found);
    devices
}

/// Parse card info from /host/asound/cards (Docker) or /proc/asound/cards (host).
/// Returns an empty map if neither file is accessible.
fn parse_asound_cards() -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let content = fs::read_to_string("/host/asound/cards")
        .or_else(|_| fs::read_to_string("/proc/asound/cards"));
    let Ok(content) = content else { return map; };

    for line in content.lines() {
        // Lines look like:  " 1 [U192k          ]: USB-Audio - UMC404HD 192k"
        if !line.starts_with(' ') || !line.contains('[') { continue; }
        let parts: Vec<&str> = line.splitn(2, '[').collect();
        if parts.len() != 2 { continue; }
        let num_str = parts[0].trim();
        let name_str = parts[1].split(']').next().unwrap_or("?").trim();
        if let Ok(n) = num_str.parse::<u32>() {
            map.insert(n, name_str.to_string());
        }
    }
    map
}
