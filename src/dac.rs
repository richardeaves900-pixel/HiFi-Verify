use std::fs;
use std::time::Duration;

fn format_pcm_fmt(fmt: &str) -> String {
    let upper = fmt.trim().to_ascii_uppercase();
    let mut desc = String::new();

    if upper.starts_with('S') {
        desc.push_str("Signed ");
    } else if upper.starts_with('U') {
        desc.push_str("Unsigned ");
    } else if upper.starts_with("FLOAT") {
        desc.push_str("Float ");
    }

    if let Some(bits) = crate::model::pcm_format_bits(&upper) {
        desc.push_str(&format!("{}-bit ", bits));
    }

    if upper.contains("_3LE") || upper.contains("_3BE") {
        desc.push_str("packed ");
    }

    if upper.ends_with("LE") {
        desc.push_str("(LE)");
    } else if upper.ends_with("BE") {
        desc.push_str("(BE)");
    }

    let trimmed = desc.trim().to_string();
    if trimmed.is_empty() {
        fmt.to_string()
    } else {
        trimmed
    }
}

fn format_dsd_fmt(fmt: &str) -> String {
    let clean = fmt.trim_start_matches("DSD_");
    let mut desc = String::new();

    let mut parts = clean.split('_');
    if let Some(container) = parts.next() {
        if let Some(bits) = container.strip_prefix('U') {
            desc.push_str(bits);
            desc.push_str("-bit");
        } else {
            desc.push_str(container);
        }
    }

    if let Some(endian) = parts.next() {
        desc.push(' ');
        desc.push_str(endian);
    }
    desc
}

fn push_rate_hz(rates: &mut Vec<u32>, hz: u32) {
    if (8000..=1_536_000).contains(&hz) && !rates.contains(&hz) {
        rates.push(hz);
    }
}

fn parse_stream0_playback(
    content: &str,
    rates: &mut Vec<u32>,
    formats: &mut Vec<String>,
    dsd_formats: &mut Vec<String>,
) {
    let mut in_playback = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line == "Playback:" {
            in_playback = true;
            continue;
        }
        if line == "Capture:" {
            in_playback = false;
            continue;
        }
        if !in_playback {
            continue;
        }

        if let Some(rates_str) = line.strip_prefix("Rates:") {
            let cleaned = rates_str.split('(').next().unwrap_or("").trim();
            if let Some((min_s, max_s)) = cleaned.split_once('-') {
                // Continuous range (e.g., "44100 - 384000 (continuous)"): emit standard rates inside range.
                if let (Ok(min_r), Ok(max_r)) =
                    (min_s.trim().parse::<u32>(), max_s.trim().parse::<u32>())
                {
                    const STANDARD_RATES: &[u32] = &[
                        44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
                        705_600, 768_000,
                    ];
                    push_rate_hz(rates, min_r);
                    for &r in STANDARD_RATES {
                        if r >= min_r && r <= max_r {
                            push_rate_hz(rates, r);
                        }
                    }
                    push_rate_hz(rates, max_r);
                }
            } else {
                for r in cleaned.split(',') {
                    if let Ok(n) = r.trim().parse::<u32>() {
                        push_rate_hz(rates, n);
                    }
                }
            }
        }

        if let Some(fmt_str) = line
            .strip_prefix("Format:")
            .or_else(|| line.strip_prefix("Formats:"))
        {
            for f in fmt_str.split(',') {
                let f = f.trim();
                if f.is_empty() {
                    continue;
                }
                if f.to_ascii_uppercase().contains("DSD") {
                    let dsd_clean = format_dsd_fmt(f);
                    if !dsd_formats.contains(&dsd_clean) {
                        dsd_formats.push(dsd_clean);
                    }
                } else {
                    let pcm_clean = format_pcm_fmt(f);
                    if !formats.contains(&pcm_clean) {
                        formats.push(pcm_clean);
                    }
                }
            }
        }
    }
}

fn read_stream0_files() -> Vec<String> {
    let mut contents = Vec::new();
    if crate::host::in_flatpak() {
        if let Ok(out) = crate::host::host_cmd(
            &["find", "/proc/asound", "-name", "stream0"],
            Duration::from_secs(2),
        ) {
            for path in String::from_utf8_lossy(&out.stdout).lines() {
                let path = path.trim();
                if path.is_empty() {
                    continue;
                }
                if let Ok(cat) = crate::host::host_cmd(&["cat", path], Duration::from_secs(1)) {
                    contents.push(String::from_utf8_lossy(&cat.stdout).into_owned());
                }
            }
        }
        return contents;
    }

    let Ok(entries) = fs::read_dir("/proc/asound") else {
        return contents;
    };
    for card in entries.flatten() {
        if !card.file_name().to_string_lossy().starts_with("card") {
            continue;
        }
        let stream0 = card.path().join("stream0");
        if let Ok(text) = fs::read_to_string(stream0) {
            contents.push(text);
        }
    }
    contents
}

pub fn get_dac_capabilities() -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut rates_hz = Vec::new();
    let mut formats = Vec::new();
    let mut dsd_formats = Vec::new();

    for content in read_stream0_files() {
        parse_stream0_playback(&content, &mut rates_hz, &mut formats, &mut dsd_formats);
    }

    // Fallback for non-USB DACs (PCI/HDA/HDMI/Thunderbolt) which do not expose stream0:
    // include the active playback hw_params so capabilities are never blank while playing.
    if rates_hz.is_empty() && formats.is_empty() {
        for hp in crate::alsa_proc::read_alsa_hw_params() {
            push_rate_hz(&mut rates_hz, hp.rate);
            let clean = format_pcm_fmt(&hp.format);
            if !formats.contains(&clean) {
                formats.push(clean);
            }
        }
    }

    rates_hz.sort_unstable();
    let rates: Vec<String> = rates_hz
        .into_iter()
        .map(crate::model::format_rate_compact)
        .collect();

    (rates, formats, dsd_formats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_playback_only_and_ignores_capture_section() {
        let stream0 = "\
FiiO K7 at usb-0000:00:14.0-1, high speed : USB Audio

Playback:
  Status: Running
  Interface 1
    Altset 1
    Format: S32_LE
    Channels: 2
    Rates: 44100, 48000, 96000, 192000, 384000
  Interface 1
    Altset 2
    Format: SPECIAL DSD_U32_BE
    Rates: 44100, 88200

Capture:
  Status: Stop
  Interface 2
    Altset 1
    Format: S16_LE
    Rates: 16000
";
        let mut rates = Vec::new();
        let mut fmts = Vec::new();
        let mut dsd = Vec::new();
        parse_stream0_playback(stream0, &mut rates, &mut fmts, &mut dsd);
        assert_eq!(rates, vec![44100, 48000, 96000, 192000, 384000, 88200]);
        assert_eq!(fmts, vec!["Signed 32-bit (LE)"]);
        assert_eq!(dsd.len(), 1);
    }

    #[test]
    fn expands_continuous_rate_ranges() {
        let stream0 = "\
Playback:
  Interface 1
    Format: S24_3LE
    Rates: 44100 - 192000 (continuous)
";
        let mut rates = Vec::new();
        let mut fmts = Vec::new();
        let mut dsd = Vec::new();
        parse_stream0_playback(stream0, &mut rates, &mut fmts, &mut dsd);
        assert!(rates.contains(&44100));
        assert!(rates.contains(&96000));
        assert!(rates.contains(&192000));
        assert_eq!(fmts, vec!["Signed 24-bit packed (LE)"]);
    }
}
