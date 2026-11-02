use crate::model::HwParams;
use std::fs;
use std::path::Path;

pub fn format_to_bits(fmt: &str) -> Option<u8> {
    match fmt.to_ascii_uppercase().as_str() {
        "S8" | "U8" => Some(8),
        "S16_LE" | "S16_BE" | "U16_LE" | "U16_BE" => Some(16),
        "S24_LE" | "S24_BE" | "S24_3LE" | "S24_3BE" => Some(24),
        "S32_LE" | "S32_BE" | "FLOAT_LE" | "FLOAT_BE" => Some(32),
        "FLOAT64_LE" | "FLOAT64_BE" => Some(64),
        _ => None,
    }
}

fn parse_hw(path: &Path, text: &str) -> Option<HwParams> {
    if text.trim().is_empty() || text.contains("closed") {
        return None;
    }
    let mut hp = HwParams {
        path: path.display().to_string(),
        ..Default::default()
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match key.as_str() {
            "format" => {
                hp.format = val.to_string();
                hp.bits = format_to_bits(val);
            }
            "rate" => {
                hp.rate = val
                    .split_whitespace()
                    .next()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0);
            }
            "channels" => hp.channels = val.parse().ok(),
            "period_size" => hp.period_size = val.parse().ok(),
            "buffer_size" => hp.buffer_size = val.parse().ok(),
            _ => {}
        }
    }
    if hp.format.is_empty() || hp.rate == 0 {
        None
    } else {
        Some(hp)
    }
}

pub fn read_alsa_hw_params() -> Vec<HwParams> {
    let mut out = Vec::with_capacity(4);
    let in_flatpak = std::path::Path::new("/.flatpak-info").exists();

    if in_flatpak {
        if let Ok(o) = crate::host::host_cmd(
            &["find", "/proc/asound", "-name", "hw_params"],
            std::time::Duration::from_secs(2),
        ) {
            let text_out = String::from_utf8_lossy(&o.stdout);
            for path in text_out.lines() {
                let path = path.trim();
                if path.is_empty() {
                    continue;
                }
                if let Ok(cat) =
                    crate::host::host_cmd(&["cat", path], std::time::Duration::from_secs(1))
                {
                    let content = String::from_utf8_lossy(&cat.stdout);
                    if let Some(hp) = parse_hw(Path::new(path), &content) {
                        out.push(hp);
                    }
                }
            }
        }
        return out;
    }

    let Ok(cards) = fs::read_dir("/proc/asound") else {
        return out;
    };
    for card in cards.flatten() {
        let card_name = card.file_name();
        if !card_name.to_string_lossy().starts_with("card") {
            continue;
        }
        let Ok(pcms) = fs::read_dir(card.path()) else {
            continue;
        };
        for pcm in pcms.flatten() {
            let pcm_name = pcm.file_name();
            let name = pcm_name.to_string_lossy();
            if !name.starts_with("pcm") || !name.ends_with('p') {
                continue;
            }
            let Ok(subs) = fs::read_dir(pcm.path()) else {
                continue;
            };
            for sub in subs.flatten() {
                if !sub.file_name().to_string_lossy().starts_with("sub") {
                    continue;
                }
                let hp_path = sub.path().join("hw_params");
                let Ok(text) = fs::read_to_string(&hp_path) else {
                    continue;
                };
                if let Some(hp) = parse_hw(&hp_path, &text) {
                    out.push(hp);
                }
            }
        }
    }
    out
}

/// Compare the ALSA hardware params against the source sample rate.
/// Returns (is_bitperfect, reason).
pub fn check_alsa_bitperfect(
    hw_params: &[crate::model::HwParams],
    source_rate: Option<u32>,
) -> (bool, String) {
    let Some(hw) = hw_params.first() else {
        return (false, "No ALSA hardware found".to_string());
    };

    let Some(src_rate) = source_rate else {
        return (false, "Source rate unknown".to_string());
    };

    if hw.rate != src_rate {
        return (
            false,
            format!("ALSA Resampled ({}Hz ➔ {}Hz)", src_rate, hw.rate),
        );
    }

    // Check for bit-depth widening (e.g. 16-bit file sent as S32_LE)
    if let Some(hw_bits) = hw.bits {
        // We don't know the source bit depth from MPRIS alone, so we can only
        // verify the rate. Bit-depth widening is not a failure per se,
        // but we flag it for transparency.
        if hw_bits > 24 {
            return (
                true,
                format!(
                    "ALSA Bit-Perfect (Rate {}Hz, Format {})",
                    hw.rate, hw.format
                ),
            );
        }
    }

    (
        true,
        format!(
            "ALSA Bit-Perfect (Rate {}Hz, Format {})",
            hw.rate, hw.format
        ),
    )
}
