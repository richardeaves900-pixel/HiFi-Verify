use super::StreamInfo;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub(crate) fn parse_mpd_audio(audio: &str) -> (Option<u32>, Option<u8>) {
    let mut parts = audio.split(':');
    let first = parts.next().unwrap_or("");
    let depth = parts.next().unwrap_or("");
    if let Some(factor) = first
        .to_ascii_lowercase()
        .strip_prefix("dsd")
        .and_then(|s| s.parse::<u32>().ok())
    {
        return (44_100u32.checked_mul(factor), Some(1));
    }
    let rate = first.parse::<u32>().ok().filter(|s| *s > 0);
    let bits = depth.parse::<u8>().ok().filter(|n| *n > 0);
    (rate, bits)
}

pub fn mpd_query() -> Option<StreamInfo> {
    let mut s =
        TcpStream::connect_timeout(&"127.0.0.1:6600".parse().ok()?, Duration::from_millis(250))
            .ok()?;
    s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let mut greet = [0u8; 256];
    let _ = s.read(&mut greet);
    s.write_all(b"status\ncurrentsong\nclose\n").ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let mut info = HashMap::new();
    for line in buf.lines() {
        if let Some((k, v)) = line.split_once(": ") {
            info.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if info.get("state").map(String::as_str) != Some("play") {
        return None;
    }
    let file = info.get("file").cloned().unwrap_or_default();
    let ext = std::path::Path::new(&file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let audio = info.get("audio").map(String::as_str).unwrap_or("");
    let (sr, bps) = parse_mpd_audio(audio);
    let codec = if audio.to_ascii_lowercase().starts_with("dsd") {
        "DSD"
    } else {
        match ext.as_str() {
            "flac" => "FLAC",
            "wav" | "wave" => "WAV",
            "dsf" => "DSD (DSF)",
            "dff" => "DSD (DFF)",
            "opus" => "OPUS",
            "ogg" => "OGG Vorbis",
            "mp3" => "MP3",
            "m4a" => "ALAC/AAC",
            _ => "Unknown codec",
        }
    };
    let artist = info.get("Artist").cloned().unwrap_or_default();
    let title = info.get("Title").cloned().unwrap_or_else(|| {
        std::path::Path::new(&file)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Some(StreamInfo {
        codec: (codec != "Unknown codec").then(|| codec.to_string()),
        track: Some(if artist.is_empty() {
            title
        } else {
            format!("{} - {}", artist, title)
        }),
        app: Some("MPD".into()),
        sr,
        bps,
        br: info
            .get("bitrate")
            .filter(|b| *b != "0")
            .map(|b| format!("{} kbps", b)),
        source_processing: None,
        source_processing_checked: false,
        mpd: true,
    })
}

// Remove # comments without deleting # inside quoted string values.
fn strip_mpd_comments(input: &str) -> String {
    let mut out = String::new();
    for line in input.lines() {
        let mut quoted = false;
        let mut escaped = false;
        for ch in line.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' && quoted {
                out.push(ch);
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = !quoted;
            }
            if ch == '#' && !quoted {
                break;
            }
            out.push(ch);
        }
        out.push('\n');
    }
    out
}

pub(crate) fn mpd_config_issues(config: &str, path: &str) -> Vec<String> {
    let mut issues = Vec::new();
    let filtered = strip_mpd_comments(config);
    let mut rest = filtered.as_str();
    while let Some(start) = rest.find("audio_output") {
        let after = &rest[start + "audio_output".len()..];
        let Some(open) = after.find('{') else { break };
        let Some(close) = after.find('}') else { break };
        if close <= open {
            break;
        }
        let block = &after[open + 1..close];
        let keys: HashMap<&str, &str> = block
            .lines()
            .filter_map(|line| {
                let mut parts = line.trim().splitn(2, char::is_whitespace);
                let key = parts.next()?;
                let value = parts.next()?.trim().trim_matches('"');
                Some((key, value))
            })
            .collect();
        if let Some(dev) = keys.get("device") {
            if !dev.starts_with("hw:") {
                issues.push(format!("{}: device \"{}\" is not a hw: device", path, dev));
            }
        }
        if let Some(mix) = keys.get("mixer_type") {
            if *mix != "none" {
                issues.push(format!(
                    "{}: mixer_type is \"{}\" — set to \"none\"",
                    path, mix
                ));
            }
        }
        if let Some(ar) = keys.get("auto_resample") {
            if matches!(ar.to_lowercase().as_str(), "yes" | "true" | "1") {
                issues.push(format!("{}: auto_resample is enabled", path));
            }
        }
        rest = &after[close + 1..];
    }
    issues
}

fn read_host_text(path: &str) -> Option<String> {
    if !crate::host::in_flatpak() {
        return std::fs::read_to_string(path).ok();
    }
    let out = crate::host::host_cmd(&["cat", path], Duration::from_secs(1)).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn mpd_config_check() -> Vec<String> {
    let mut issues = Vec::new();
    let mut found = false;
    let mut paths = vec!["/etc/mpd.conf".to_string()];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(format!("{}/.config/mpd/mpd.conf", home));
    }
    for p in &paths {
        let Some(text) = read_host_text(p) else {
            continue;
        };
        found = true;
        issues.extend(mpd_config_issues(&text, p));
    }
    if !found {
        return vec!["No MPD config found".into()];
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mpd_audio_formats() {
        assert_eq!(parse_mpd_audio("48000:f:2"), (Some(48000), None));
        assert_eq!(parse_mpd_audio("dsd64:2"), (Some(2_822_400), Some(1)));
    }

    #[test]
    fn mpd_comments_do_not_create_fake_settings() {
        let cfg = "# audio_output {\n# device \"default\"\n# }\naudio_output {\n type \"alsa\"\n device \"hw:0,0\"\n # mixer_type \"hardware\"\n mixer_type \"none\"\n}\n";
        assert!(mpd_config_issues(cfg, "test").is_empty());
    }
}
