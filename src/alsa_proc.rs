use crate::model::{pcm_format_bits, HwParams};
use std::fs;
use std::path::Path;
use std::time::Duration;

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
                hp.bits = pcm_format_bits(val);
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

    if crate::host::in_flatpak() {
        // Filter strictly to playback PCM subdevices (pcm*p), never capture (pcm*c).
        if let Ok(o) = crate::host::host_cmd(
            &["find", "/proc/asound", "-path", "*/pcm*p/sub*/hw_params"],
            Duration::from_secs(2),
        ) {
            let text_out = String::from_utf8_lossy(&o.stdout);
            for path in text_out.lines() {
                let path = path.trim();
                if path.is_empty() {
                    continue;
                }
                if let Ok(cat) = crate::host::host_cmd(&["cat", path], Duration::from_secs(1)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_hw_params_and_ignores_closed() {
        let open = "\
access: MMAP_INTERLEAVED
format: S32_LE
subformat: STD
channels: 2
rate: 96000 (96000/1)
period_size: 512
buffer_size: 1024
";
        let parsed = parse_hw(Path::new("/proc/asound/card1/pcm0p/sub0/hw_params"), open).unwrap();
        assert_eq!(parsed.format, "S32_LE");
        assert_eq!(parsed.bits, Some(32));
        assert_eq!(parsed.rate, 96000);
        assert_eq!(parsed.channels, Some(2));
        assert_eq!(parsed.period_size, Some(512));
        assert_eq!(parsed.buffer_size, Some(1024));

        assert!(parse_hw(
            Path::new("/proc/asound/card0/pcm0p/sub0/hw_params"),
            "closed\n"
        )
        .is_none());
    }
}
