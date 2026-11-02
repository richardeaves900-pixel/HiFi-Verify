use crate::model::Sink;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    pub codec: Option<String>,
    pub track: Option<String>,
    pub app: Option<String>,
    pub sr: Option<u32>,
    pub bps: Option<u8>,
    pub br: Option<String>,
    pub mpd: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StreamNode {
    pub app_name: String,
    #[allow(dead_code)]
    pub node_name: String,
    pub media_name: String,
    pub pid: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub struct PwGraph {
    pub sinks: Vec<Sink>,
    pub streams: Vec<StreamNode>,
    pub processing: Vec<String>,
    pub driver_rate: Option<u32>,
    pub driver_quantum: Option<u32>,
}

// Identify the *producer* of a running playback stream. The /proc comm field
// is a last resort: Firefox subprocess names may describe a content process
// rather than the Firefox application, and sandboxed PIDs may be inaccessible.
fn playback_app_name(props: &serde_json::Value, pid: Option<i32>) -> String {
    let name = [
        "application.name",
        "application.process.binary",
        "client.name",
    ]
    .iter()
    .filter_map(|key| props.get(*key).and_then(|v| v.as_str()))
    .map(str::trim)
    .find(|s| {
        !s.is_empty()
            && !matches!(
                s.to_ascii_lowercase().as_str(),
                "pipewire" | "pipewire-pulse" | "unknown"
            )
    })
    .map(str::to_owned)
    .or_else(|| {
        pid.and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
    .unwrap_or_else(|| "Unknown application".to_string());

    match name.to_ascii_lowercase().as_str() {
        "firefox" | "mozilla firefox" | "firefox-bin" => "Firefox".to_string(),
        _ => name,
    }
}

fn is_playback_candidate(n: &StreamNode) -> bool {
    let id = format!("{} {}", n.app_name, n.node_name).to_ascii_lowercase();
    !["easyeffects", "ee_", "filter-chain", "cava", "hifi-verify"]
        .iter()
        .any(|term| id.contains(term))
}

impl PwGraph {
    pub fn load() -> Self {
        let mut g = Self::default();
        let Ok(out) = crate::host::host_cmd(&["pw-dump"], Duration::from_secs(3)) else {
            return g;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
            return g;
        };
        let Some(arr) = v.as_array() else { return g };

        let hints = [
            "easyeffects",
            "carla",
            "jamesdsp",
            "filter-chain",
            "calf",
            "lsp-plugins",
            "compressor",
            "equalizer",
        ];

        for obj in arr {
            let Some(info) = obj.get("info") else {
                continue;
            };
            let Some(props) = info.get("props") else {
                continue;
            };
            let media_class = props
                .get("media.class")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let state = info.get("state").and_then(|x| x.as_str()).unwrap_or("");
            let oid = obj.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
            let app = props
                .get("application.name")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let node = props
                .get("node.name")
                .and_then(|x| x.as_str())
                .unwrap_or("");

            if state == "running" && (media_class == "Audio/Sink" || media_class == "Audio/Source")
            {
                let rate = props.get("audio.rate").and_then(|x| {
                    x.as_u64()
                        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                });
                if let Some(r) = rate {
                    if g.driver_rate.is_none() {
                        g.driver_rate = Some(r as u32);
                    }
                }
                let quantum_direct = props
                    .get("audio.quantum")
                    .and_then(|x| x.as_u64())
                    .map(|n| n as u32);
                let quantum_node = props
                    .get("node.quantum")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.split('/').next().and_then(|n| n.parse::<u32>().ok()));
                if let Some(q) = quantum_direct.or(quantum_node) {
                    if g.driver_quantum.is_none() {
                        g.driver_quantum = Some(q);
                    }
                }
            }

            if media_class == "Audio/Sink" {
                let desc = props
                    .get("node.description")
                    .and_then(|x| x.as_str())
                    .or_else(|| props.get("node.nick").and_then(|x| x.as_str()))
                    .unwrap_or(node);
                g.sinks.push(Sink {
                    id: oid.to_string(),
                    description: format!("{} (ID:{})", desc, oid),
                });
            }

            if media_class.contains("Stream/Output/Audio") && state == "running" {
                let pid = props.get("application.process.id").and_then(|x| {
                    x.as_u64()
                        .map(|n| n as i32)
                        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                });

                let resolved_app = playback_app_name(props, pid);

                g.streams.push(StreamNode {
                    app_name: resolved_app,
                    node_name: node.to_string(),
                    media_name: props
                        .get("media.name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    pid,
                });
            }

            let low = format!("{} {}", app, node).to_ascii_lowercase();
            if state == "running" && hints.iter().any(|h| low.contains(h)) {
                g.processing.push(if app.is_empty() {
                    node.to_string()
                } else {
                    app.to_string()
                });
            }
        }
        g
    }

    pub fn primary_stream(&self) -> Option<StreamInfo> {
        // Visualisers and DSP helper nodes are not the audio-producing app.
        let first_valid = self.streams.iter().find(|n| is_playback_candidate(n));

        first_valid.map(|n| {
            let low = format!("{} {}", n.app_name, n.node_name).to_lowercase();
            let inferred_codec = if low.contains("feishin") || low.contains("mpv") {
                Some("FLAC / Stream".to_string())
            } else if low.contains("high-tide") || low.contains("tidal") {
                Some("FLAC (Tidal)".to_string())
            } else if low.contains("spotify") {
                Some("Ogg Vorbis".to_string())
            } else {
                None
            };

            StreamInfo {
                codec: n.pid.and_then(sniff_pid_files).or(inferred_codec),
                track: Some(if n.media_name.is_empty() {
                    n.app_name.clone()
                } else {
                    n.media_name.clone()
                }),
                app: Some(n.app_name.clone()),
                sr: None,
                bps: None,
                br: None,
                mpd: false,
            }
        })
    }
}

// A MPRIS media-player ID such as Instance_1_50086 is not an application
// identity. PipeWire owns application attribution; MPRIS only enriches tracks.
fn same_app(a: &str, b: &str) -> bool {
    let key = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    !a.is_empty() && key(a) == key(b)
}

fn generic_player_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "player" || n == "unknown player" || n.starts_with("instance_")
}

// Pure selector, so both behaviour and failure modes can be unit-tested.
fn select_stream(
    graph: &PwGraph,
    mpd: Option<StreamInfo>,
    mpris: Option<StreamInfo>,
) -> Option<StreamInfo> {
    if let Some(mut live) = graph.primary_stream() {
        // MPD reports genuine source format and depth, where available.
        if live
            .app
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("mpd"))
        {
            return mpd.or(Some(live));
        }

        if let Some(media) = mpris {
            let player_name = media.app.as_deref().unwrap_or("");
            let matches = live
                .app
                .as_deref()
                .is_some_and(|app| same_app(app, player_name));
            let only_producer = graph
                .streams
                .iter()
                .filter(|n| is_playback_candidate(n))
                .count()
                == 1;
            // Generic media-player IDs are safe to associate only if there is
            // one eligible running playback producer. Do not invent codecs.
            if matches || (only_producer && generic_player_name(player_name)) {
                if let Some(track) = media
                    .track
                    .filter(|s| !s.is_empty() && !generic_player_name(s))
                {
                    live.track = Some(track);
                }
                live.codec = media.codec.or(live.codec);
                live.sr = media.sr.or(live.sr);
                live.bps = media.bps.or(live.bps);
                live.br = media.br.or(live.br);
            }
        }
        return Some(live);
    }

    // No PipeWire playback node seen: preserve the existing MPD/MPRIS fallback.
    mpd.or(mpris)
}

pub fn detect_active_stream(graph: &PwGraph) -> Option<StreamInfo> {
    match graph.primary_stream() {
        Some(ref pw)
            if pw
                .app
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("mpd")) =>
        {
            select_stream(graph, mpd_query(), None)
        }
        Some(_) => select_stream(graph, None, mpris_query()),
        None => select_stream(graph, mpd_query(), mpris_query()),
    }
}

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

    let mut info = std::collections::HashMap::new();
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
    let codec = match ext.as_str() {
        "flac" => "FLAC",
        "wav" | "wave" => "WAV",
        "dsf" => "DSD (DSF)",
        "dff" => "DSD (DFF)",
        "opus" => "OPUS",
        "ogg" => "OGG Vorbis",
        "mp3" => "MP3",
        "m4a" => "ALAC/AAC",
        _ => "PCM",
    };
    let (sr, bps) = info
        .get("audio")
        .and_then(|a| {
            let mut it = a.split(':');
            let sr = it.next()?.parse().ok();
            let bps = it.next().and_then(|b| b.parse().ok());
            Some((sr, bps))
        })
        .unwrap_or((None, None));
    let artist = info.get("Artist").cloned().unwrap_or_default();
    let title = info.get("Title").cloned().unwrap_or_else(|| {
        std::path::Path::new(&file)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Some(StreamInfo {
        codec: Some(codec.into()),
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
        mpd: true,
    })
}

fn friendly_app_name(bus_name: &str) -> String {
    let s = bus_name.to_lowercase();
    if s.contains("high-tide") || s.contains("hightide") {
        return "High Tide".to_string();
    }
    if s.contains("feishin") {
        return "Feishin".to_string();
    }
    if s.contains("spotify") {
        return "Spotify".to_string();
    }
    if s.contains("vlc") {
        return "VLC".to_string();
    }
    if s.contains("mpv") {
        return "mpv".to_string();
    }
    if s.contains("rhythmbox") {
        return "Rhythmbox".to_string();
    }
    if s.contains("strawberry") {
        return "Strawberry".to_string();
    }
    if s.contains("elisa") {
        return "Elisa".to_string();
    }
    // Fallback: last meaningful segment of the bus name
    bus_name
        .trim_start_matches("org.mpris.MediaPlayer2.")
        .split('.')
        .filter(|p| {
            let lower = p.to_lowercase();
            !matches!(
                lower.as_str(),
                "io" | "com" | "org" | "net" | "github" | "gitlab" | "instance"
            ) && !lower.chars().all(|c| c.is_ascii_digit())
        })
        .last()
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => "Player".into(),
            }
        })
        .unwrap_or_else(|| "Player".to_string())
}

pub fn mpris_query() -> Option<StreamInfo> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;

    // gio 0.22 signature:
    //   call_sync(
    //     bus_name:      Option<&str>,
    //     object_path:   &str,
    //     interface:     &str,
    //     method:        &str,
    //     parameters:    Option<&Variant>,
    //     reply_type:    Option<&VariantTy>,
    //     flags, timeout, cancellable,
    //   )
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
            Some(glib::VariantTy::new("(as)").ok()?),
            gio::DBusCallFlags::NONE,
            250,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let names: Vec<String> = reply.get::<(Vec<String>,)>()?.0;

    for name in names
        .iter()
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
    {
        let Ok(props) = bus.call_sync(
            Some(name.as_str()),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&glib::Variant::from(("org.mpris.MediaPlayer2.Player",))),
            Some(glib::VariantTy::new("(a{sv})").ok()?),
            gio::DBusCallFlags::NONE,
            250,
            gio::Cancellable::NONE,
        ) else {
            continue;
        };
        let Some(dict) = props.get::<(std::collections::HashMap<String, glib::Variant>,)>() else {
            continue;
        };
        let map = dict.0;
        if map
            .get("PlaybackStatus")
            .and_then(|v| v.get::<String>())
            .as_deref()
            != Some("Playing")
        {
            continue;
        }

        let mut codec: Option<String> = None;
        let mut sr: Option<u32> = None;
        let mut bps: Option<u8> = None;
        let lower_name = name.to_lowercase();
        if lower_name.contains("high-tide") || lower_name.contains("hightide") {
            codec = Some("FLAC (Tidal HiRes)".into());
        } else if lower_name.contains("feishin") {
            codec = Some("FLAC / Stream".into());
        }

        let mut track = name
            .trim_start_matches("org.mpris.MediaPlayer2.")
            .split('.')
            .next()
            .unwrap_or("Player")
            .to_string();

        if let Some(meta) = map
            .get("Metadata")
            .and_then(|v| v.get::<std::collections::HashMap<String, glib::Variant>>())
        {
            if let Some(t) = meta.get("xesam:title").and_then(|v| v.get::<String>()) {
                track = t;
            }
            if let Some(url) = meta.get("xesam:url").and_then(|v| v.get::<String>()) {
                let path = url.strip_prefix("file://").unwrap_or(&url);
                if path.ends_with(".flac") {
                    codec = Some("FLAC".into());
                    if let Some((s, b, _)) = flac_header(path) {
                        sr = Some(s);
                        bps = Some(b);
                    }
                }
            }
        }

        return Some(StreamInfo {
            codec,
            track: Some(track),
            app: Some(friendly_app_name(name)),
            sr,
            bps,
            br: None,
            mpd: false,
        });
    }
    None
}

pub fn mpd_config_check() -> Vec<String> {
    let mut issues = Vec::new();
    let mut found = false;
    let mut paths = vec!["/etc/mpd.conf".to_string()];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(format!("{}/.config/mpd/mpd.conf", home));
    }
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        found = true;
        let mut rest = text.as_str();
        while let Some(start) = rest.find("audio_output") {
            let after = &rest[start..];
            let Some(open) = after.find('{') else { break };
            let Some(close) = after.find('}') else { break };
            if close <= open {
                break;
            }
            let block = &after[open + 1..close];
            if let Some(dev) = block
                .split("device")
                .nth(1)
                .and_then(|s| s.split('"').nth(1))
            {
                if !dev.starts_with("hw:") {
                    issues.push(format!("{}: device \"{}\" is not a hw: device", p, dev));
                }
            }
            if let Some(mix) = block
                .split("mixer_type")
                .nth(1)
                .and_then(|s| s.split('"').nth(1))
            {
                if mix != "none" {
                    issues.push(format!(
                        "{}: mixer_type is \"{}\" — set to \"none\"",
                        p, mix
                    ));
                }
            }
            if let Some(ar) = block
                .split("auto_resample")
                .nth(1)
                .and_then(|s| s.split('"').nth(1))
            {
                if matches!(ar.to_lowercase().as_str(), "yes" | "true" | "1") {
                    issues.push(format!("{}: auto_resample is enabled", p));
                }
            }
            rest = &after[close + 1..];
        }
    }
    if !found {
        return vec!["No MPD config found".into()];
    }
    issues
}

pub fn sniff_pid_files(pid: i32) -> Option<String> {
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

pub fn send_mpris_command(command: &str) {
    let bus = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(b) => b,
        Err(_) => return,
    };

    let reply = match bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        None,
        Some(glib::VariantTy::new("(as)").ok().unwrap()),
        gio::DBusCallFlags::NONE,
        250,
        gio::Cancellable::NONE,
    ) {
        Ok(r) => r,
        Err(_) => return,
    };

    let names: Vec<String> = reply.get::<(Vec<String>,)>().unwrap().0;
    // Find the first active MPRIS player
    if let Some(player) = names
        .iter()
        .find(|n| n.starts_with("org.mpris.MediaPlayer2."))
    {
        let _ = bus.call_sync(
            Some(player.as_str()),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            command,
            None,
            None,
            gio::DBusCallFlags::NONE,
            250,
            gio::Cancellable::NONE,
        );
    }
}

#[cfg(test)]
mod app_attribution_tests {
    use super::*;

    fn node(app: &str) -> StreamNode {
        StreamNode {
            app_name: app.to_string(),
            node_name: app.to_string(),
            media_name: format!("{app} audio"),
            pid: None,
        }
    }

    #[test]
    fn app_name_prefers_pipewire_identity_to_process_name() {
        let props = serde_json::json!({
            "application.name": "Firefox",
            "application.process.binary": "Isolated Web Co",
        });
        assert_eq!(playback_app_name(&props, None), "Firefox");
    }

    #[test]
    fn binary_fallback_identifies_firefox() {
        let props = serde_json::json!({ "application.process.binary": "firefox" });
        assert_eq!(playback_app_name(&props, None), "Firefox");
    }

    #[test]
    fn firefox_wins_over_anonymous_youtube_mpris_player() {
        let graph = PwGraph {
            streams: vec![node("cava"), node("Firefox")],
            ..Default::default()
        };
        let mpris = StreamInfo {
            app: Some("Instance_1_50086".to_string()),
            track: Some("YouTube video title".to_string()),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(mpris)).unwrap();
        assert_eq!(chosen.app.as_deref(), Some("Firefox"));
        assert_eq!(chosen.track.as_deref(), Some("YouTube video title"));
        assert!(chosen.codec.is_none(), "browser codec must not be guessed");
    }

    #[test]
    fn known_mpris_for_another_app_cannot_rename_firefox() {
        let graph = PwGraph {
            streams: vec![node("Firefox")],
            ..Default::default()
        };
        let mpris = StreamInfo {
            app: Some("VLC".to_string()),
            track: Some("Unrelated track".to_string()),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(mpris)).unwrap();
        assert_eq!(chosen.app.as_deref(), Some("Firefox"));
        assert_ne!(chosen.track.as_deref(), Some("Unrelated track"));
    }

    #[test]
    fn no_media_sources_reports_idle() {
        let graph = PwGraph {
            streams: vec![node("cava")],
            ..Default::default()
        };
        assert!(select_stream(&graph, None, None).is_none());
    }
}
