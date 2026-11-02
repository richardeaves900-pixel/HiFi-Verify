use std::time::Duration;

/// Read the current PipeWire sink volume, preserving query failures.
pub fn try_get_volume() -> Option<f64> {
    let out = crate::host::host_cmd(
        &["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"],
        Duration::from_millis(200),
    )
    .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Example: "Volume: 0.65 [MUTED]"
    text.split_whitespace().nth(1)?.parse::<f64>().ok()
}

/// Retained for callers that need a fallback, such as volume button actions.
pub fn get_volume() -> f64 {
    try_get_volume().unwrap_or(1.0)
}

pub fn set_volume(vol: f64) {
    let vol = vol.clamp(0.0, 1.5); // Allow up to 150% for some DACs
    let vol_str = format!("{:.2}", vol);
    let _ = crate::host::host_cmd(
        &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &vol_str],
        Duration::from_millis(200),
    );
}

pub fn change_volume(delta: f64) {
    let current = get_volume();
    set_volume(current + delta);
}
