use std::io::Read;
use std::path::PathBuf;

pub fn flac_header(path: &str) -> Option<(u32, u8, u8)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 42];
    f.read_exact(&mut buf).ok()?;
    if &buf[..4] != b"fLaC" {
        return None;
    }
    let val = u64::from_be_bytes(buf[18..26].try_into().ok()?);
    let sr = ((val >> 44) & 0xFFFFF) as u32;
    let ch = (((val >> 41) & 0x7) + 1) as u8;
    let bps = (((val >> 36) & 0x1F) + 1) as u8;
    if sr > 0 {
        Some((sr, bps, ch))
    } else {
        None
    }
}

pub fn local_flac_path(url: &str) -> Option<PathBuf> {
    use gio::prelude::FileExt;
    if !url.starts_with("file://") {
        return None;
    }
    let path = gio::File::for_uri(url).path()?;
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("flac"))
    {
        Some(path)
    } else {
        None
    }
}

pub fn sniff_pid_files(pid: i32) -> Option<String> {
    if crate::host::in_flatpak() {
        return None;
    }
    let dir = std::fs::read_dir(format!("/proc/{}/fd", pid)).ok()?;
    for entry in dir.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let low = target.to_string_lossy().to_ascii_lowercase();
        for (ext, label) in [
            (".flac", "FLAC"),
            (".wav", "WAV"),
            (".dsf", "DSD (DSF)"),
            (".dff", "DSD (DFF)"),
            (".opus", "OPUS"),
            (".ogg", "OGG Vorbis"),
            (".mp3", "MP3"),
            (".m4a", "ALAC/AAC"),
        ] {
            if low.ends_with(ext) {
                return Some(label.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_is_percent_decoded_and_remote_uri_is_rejected() {
        let path = local_flac_path("file:///home/user/Music/01%20Track%20%C3%A9.flac").unwrap();
        assert_eq!(path.to_string_lossy(), "/home/user/Music/01 Track é.flac");
        assert!(local_flac_path("https://example.com/stream.flac").is_none());
    }
}
