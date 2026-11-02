use std::time::Duration;

pub fn get_volume() -> f64 {
    let out = crate::host::host_cmd(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], Duration::from_millis(200));
    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        // Output format is: "Volume: 0.65" or "Volume: 0.65 [MUTED]"
        if let Some(val) = text.split_whitespace().nth(1) {
            return val.parse().unwrap_or(1.0);
        }
    }
    1.0
}

pub fn set_volume(vol: f64) {
    let vol = vol.clamp(0.0, 1.5); // Allow up to 150% for some DACs
    let vol_str = format!("{:.2}", vol);
    let _ = crate::host::host_cmd(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &vol_str], Duration::from_millis(200));
}

pub fn change_volume(delta: f64) {
    let current = get_volume();
    set_volume(current + delta);
}
