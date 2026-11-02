use std::time::Duration;

fn format_pcm_fmt(fmt: &str) -> String {
    let mut desc = String::new();

    if fmt.starts_with("S") {
        desc.push_str("Signed ");
    } else if fmt.starts_with("U") {
        desc.push_str("Unsigned ");
    } else if fmt.starts_with("FLOAT") {
        desc.push_str("Float ");
    }

    // Extract bit depth (first two digits)
    let digits: String = fmt.chars().filter(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let bits = if digits.len() >= 2 {
            &digits[0..2]
        } else {
            &digits
        };
        desc.push_str(bits);
        desc.push_str("-bit ");
    }

    if fmt.ends_with("_LE") {
        desc.push_str("(LE)");
    } else if fmt.ends_with("_BE") {
        desc.push_str("(BE)");
    }

    desc = desc.trim().to_string();
    if desc.is_empty() {
        fmt.to_string()
    } else {
        desc
    }
}

fn format_dsd_fmt(fmt: &str) -> String {
    let clean = fmt.trim_start_matches("DSD_");
    let mut desc = String::new();

    let mut parts = clean.split('_');
    if let Some(container) = parts.next() {
        if container.starts_with('U') {
            let bits = &container[1..];
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

pub fn get_dac_capabilities() -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut rates = Vec::new();
    let mut formats = Vec::new();
    let mut dsd_formats = Vec::new();

    let in_flatpak = std::path::Path::new("/.flatpak-info").exists();
    let find_cmd = if in_flatpak {
        crate::host::host_cmd(
            &["find", "/proc/asound", "-name", "stream0"],
            Duration::from_millis(500),
        )
    } else {
        std::process::Command::new("find")
            .args(["/proc/asound", "-name", "stream0"])
            .output()
    };

    if let Ok(out) = find_cmd {
        let text = String::from_utf8_lossy(&out.stdout);
        for path in text.lines() {
            if path.trim().is_empty() {
                continue;
            }
            let cat_cmd = if in_flatpak {
                crate::host::host_cmd(&["cat", path], Duration::from_millis(500))
            } else {
                std::process::Command::new("cat").arg(path).output()
            };

            if let Ok(cat_out) = cat_cmd {
                let content = String::from_utf8_lossy(&cat_out.stdout);
                for line in content.lines() {
                    let line = line.trim();

                    // Parse Sample Rates
                    if line.starts_with("Rates:") {
                        let rates_str = line.trim_start_matches("Rates:").trim();
                        for r in rates_str.split(',') {
                            let r = r.trim();
                            if !r.is_empty() {
                                if let Ok(n) = r.parse::<u32>() {
                                    let khz = n as f64 / 1000.0;
                                    let txt = if khz.fract() == 0.0 {
                                        format!("{}kHz", khz as u32)
                                    } else {
                                        format!("{:.1}kHz", khz)
                                    };
                                    if !rates.contains(&txt) {
                                        rates.push(txt);
                                    }
                                }
                            }
                        }
                    }

                    // Parse PCM & DSD Formats
                    if line.starts_with("Format:") || line.starts_with("Formats:") {
                        let fmt_str = line.split(':').nth(1).unwrap_or("").trim();
                        for f in fmt_str.split(',') {
                            let f = f.trim();
                            if !f.is_empty() {
                                if f.contains("DSD") {
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
            }
        }
    }

    // Sort rates numerically based on kHz value
    rates.sort_by(|a, b| {
        let parse = |s: &str| s.trim_end_matches("kHz").parse::<f64>().unwrap_or(0.0);
        parse(a)
            .partial_cmp(&parse(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (rates, formats, dsd_formats)
}
