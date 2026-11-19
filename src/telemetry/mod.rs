pub mod mpd;
pub mod mpris;
pub mod proc_sniff;
pub mod pw_graph;

pub use mpd::{mpd_config_check, mpd_query};
pub use mpris::{mpris_query_for, send_mpris_command};
pub use pw_graph::{observed_output_clock, PcmPathAssessment, PwGraph};

#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    pub codec: Option<String>,
    pub track: Option<String>,
    pub app: Option<String>,
    pub sr: Option<u32>,
    pub bps: Option<u8>,
    pub br: Option<String>,
    /// Source-side processing explicitly reported by the player.
    pub source_processing: Option<String>,
    /// True only when the player reported both normalisation and *raw* gain.
    pub source_processing_checked: bool,
    pub mpd: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StreamNode {
    pub id: u64,
    pub app_name: String,
    pub node_name: String,
    pub media_name: String,
    pub pid: Option<i32>,
}

// Recognise stable Flatpak identities before generic Python process names.
// Do not infer a codec or a TIDAL source sample rate from an application ID.
pub(crate) fn known_application_id(props: &serde_json::Value) -> Option<&'static str> {
    ["pipewire.access.portal.app_id", "application.id"]
        .iter()
        .filter_map(|key| props.get(*key).and_then(|v| v.as_str()))
        .find_map(|id| match id.to_ascii_lowercase().as_str() {
            "io.github.nokse22.high-tide" | "io.github.nokse22.hightide" => Some("High Tide"),
            _ => None,
        })
}

pub(crate) fn uninformative_app_name(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    name.is_empty()
        || matches!(
            name.as_str(),
            "unknown"
                | "unknown application"
                | "python"
                | "isolated web co"
                | "audioipc server"
                | "web content"
                | "plugin-container"
        )
        || name.starts_with("python3")
}

pub(crate) fn process_pid(props: &serde_json::Value) -> Option<i32> {
    ["application.process.id", "pipewire.sec.pid"]
        .iter()
        .filter_map(|key| props.get(*key))
        .find_map(|v| {
            v.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .or_else(|| v.as_str().and_then(|s| s.parse::<i32>().ok()))
                .filter(|pid| *pid > 0)
        })
}

pub(crate) fn resolve_playback_app(
    node_props: &serde_json::Value,
    client_props: Option<&serde_json::Value>,
    pid: Option<i32>,
) -> String {
    if let Some(known) =
        known_application_id(node_props).or_else(|| client_props.and_then(known_application_id))
    {
        return known.to_string();
    }
    let node = playback_app_name(node_props, None);
    if !uninformative_app_name(&node) {
        return node;
    }
    if let Some(client) = client_props {
        let name = playback_app_name(client, None);
        if !uninformative_app_name(&name) {
            return name;
        }
    }
    let fallback = playback_app_name(&serde_json::Value::Null, pid);
    if !uninformative_app_name(&fallback) {
        fallback
    } else {
        node
    }
}

pub(crate) fn playback_app_name(props: &serde_json::Value, pid: Option<i32>) -> String {
    if let Some(known) = known_application_id(props) {
        return known.to_string();
    }
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
        if crate::host::in_flatpak() {
            return None;
        }
        pid.and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
    .unwrap_or_else(|| "Unknown application".to_string());

    match name.to_ascii_lowercase().as_str() {
        "firefox" | "mozilla firefox" | "firefox-bin" => "Firefox".to_string(),
        "mpd" | "music player daemon" => "MPD".to_string(),
        _ => name,
    }
}

pub(crate) fn is_dsp_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "easyeffects",
        "ee_",
        "filter-chain",
        "cava",
        "hifi-verify",
        "jamesdsp",
        "carla",
        "calf",
        "lsp-plugins",
        "lsp_",
        "compressor",
        "equalizer",
    ]
    .iter()
    .any(|part| name.contains(part))
}

/// Map a PipeWire node/app pair to a human-readable DSP label, returning `None`
/// for non-DSP nodes and passive visualiser/monitor nodes (`cava`, `hifi-verify`).
pub(crate) fn dsp_label(app: &str, node: &str) -> Option<String> {
    let combined = format!("{app} {node}");
    if !is_dsp_name(&combined) {
        return None;
    }
    let lower = combined.to_ascii_lowercase();
    if lower.contains("cava") || lower.contains("hifi-verify") {
        return None;
    }
    let label = if lower.contains("easyeffects") || lower.contains("ee_") {
        "EasyEffects".to_string()
    } else if lower.contains("jamesdsp") {
        "JamesDSP".to_string()
    } else if lower.contains("carla") {
        "Carla".to_string()
    } else if lower.contains("calf") {
        "Calf".to_string()
    } else if lower.contains("lsp-") || lower.contains("lsp_") {
        "LSP Plugins".to_string()
    } else if !app.is_empty() {
        app.to_string()
    } else {
        node.to_string()
    };
    Some(label)
}

pub(crate) fn is_playback_candidate(n: &StreamNode) -> bool {
    !is_dsp_name(&format!("{} {}", n.app_name, n.node_name))
}

pub(crate) fn is_mpd_app(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "mpd" | "music player daemon"
    )
}

pub(crate) fn same_app(a: &str, b: &str) -> bool {
    let key = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    !a.is_empty() && key(a) == key(b)
}

pub(crate) fn generic_player_name(name: &str) -> bool {
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
        if live.app.as_deref().is_some_and(is_mpd_app) {
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
            let mpris_identifies_unknown = only_producer
                && !generic_player_name(player_name)
                && !player_name.trim().is_empty()
                && live.app.as_deref().is_none_or(uninformative_app_name);
            let anonymous = only_producer && generic_player_name(player_name);
            if matches || anonymous || mpris_identifies_unknown {
                if mpris_identifies_unknown {
                    live.app = Some(player_name.to_string());
                }
                if let Some(track) = media
                    .track
                    .filter(|s| !s.is_empty() && !generic_player_name(s))
                {
                    live.track = Some(track);
                }
                if !anonymous {
                    live.codec = media.codec.or(live.codec);
                    live.sr = media.sr.or(live.sr);
                    live.bps = media.bps.or(live.bps);
                    live.br = media.br.or(live.br);
                    live.source_processing = media.source_processing.or(live.source_processing);
                    live.source_processing_checked |= media.source_processing_checked;
                }
            }
        }
        return Some(live);
    }

    mpd.or(mpris)
}

pub fn detect_active_stream(graph: &PwGraph) -> Option<StreamInfo> {
    match graph.primary_stream() {
        Some(ref pw) if pw.app.as_deref().is_some_and(is_mpd_app) => {
            select_stream(graph, mpd_query(), None)
        }
        Some(ref pw) => select_stream(graph, None, mpris_query_for(pw.app.as_deref())),
        None => select_stream(graph, mpd_query(), mpris_query_for(None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(app: &str) -> StreamNode {
        StreamNode {
            id: 0,
            app_name: app.to_string(),
            node_name: app.to_string(),
            media_name: format!("{app} audio"),
            pid: None,
        }
    }

    #[test]
    fn app_name_prefers_pipewire_identity_to_process_name() {
        let props = json!({
            "application.name": "Firefox",
            "application.process.binary": "Isolated Web Co",
        });
        assert_eq!(playback_app_name(&props, None), "Firefox");
    }

    #[test]
    fn binary_fallback_identifies_firefox() {
        let props = json!({ "application.process.binary": "firefox" });
        assert_eq!(playback_app_name(&props, None), "Firefox");
    }

    #[test]
    fn normalizes_mpd_identity() {
        assert_eq!(
            playback_app_name(&json!({"application.name":"Music Player Daemon"}), None),
            "MPD"
        );
    }

    #[test]
    fn node_fallback_does_not_override_client_application() {
        let node = json!({ "node.name": "firefox", "application.process.id": 999999 });
        let client = json!({ "application.name": "Firefox", "pipewire.sec.pid": 42 });
        assert_eq!(
            resolve_playback_app(&node, Some(&client), Some(999999)),
            "Firefox"
        );
        assert_eq!(process_pid(&client), Some(42));
        let child = json!({ "application.name": "Isolated Web Co" });
        assert_eq!(resolve_playback_app(&child, Some(&client), None), "Firefox");
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
        assert!(chosen.codec.is_none());
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
    fn high_tide_flatpak_id_overrides_generic_python() {
        let props = json!({
            "application.name": "python3.13",
            "pipewire.access.portal.app_id": "io.github.nokse22.high-tide",
        });
        assert_eq!(playback_app_name(&props, None), "High Tide");
    }

    #[test]
    fn high_tide_client_identity_is_used_when_node_name_is_missing() {
        let node = json!({ "node.name": "python3.13" });
        let client = json!({
            "application.name": "python3.13",
            "application.id": "io.github.nokse22.high-tide",
        });
        assert_eq!(
            resolve_playback_app(&node, Some(&client), None),
            "High Tide"
        );
    }

    #[test]
    fn sole_python_playback_stream_can_use_active_high_tide_mpris() {
        let graph = PwGraph {
            streams: vec![node("python3.13")],
            ..Default::default()
        };
        let mpris = StreamInfo {
            app: Some("High Tide".into()),
            track: Some("Rattle The Cage".into()),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(mpris)).unwrap();
        assert_eq!(chosen.app.as_deref(), Some("High Tide"));
        assert_eq!(chosen.track.as_deref(), Some("Rattle The Cage"));
        assert_eq!(chosen.sr, None);
        assert_eq!(chosen.codec, None);
    }

    #[test]
    fn unrelated_player_cannot_rename_python_when_multiple_apps_play() {
        let graph = PwGraph {
            streams: vec![node("python3.13"), node("Firefox")],
            ..Default::default()
        };
        let mpris = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(mpris)).unwrap();
        assert_eq!(chosen.app.as_deref(), Some("python3.13"));
    }

    #[test]
    fn matching_player_preserves_real_source_metadata() {
        let graph = PwGraph {
            streams: vec![node("High Tide")],
            ..Default::default()
        };
        let media = StreamInfo {
            app: Some("High Tide".into()),
            codec: Some("FLAC".into()),
            sr: Some(44100),
            bps: Some(16),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(media)).unwrap();
        assert_eq!(chosen.codec.as_deref(), Some("FLAC"));
        assert_eq!(chosen.sr, Some(44100));
        assert_eq!(chosen.bps, Some(16));
    }

    #[test]
    fn unrelated_player_cannot_supply_source_format() {
        let graph = PwGraph {
            streams: vec![node("High Tide")],
            ..Default::default()
        };
        let media = StreamInfo {
            app: Some("Feishin".into()),
            codec: Some("FLAC".into()),
            sr: Some(192000),
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(media)).unwrap();
        assert_ne!(chosen.sr, Some(192000));
    }

    #[test]
    fn real_source_metadata_carries_clean_gain_evidence() {
        let graph = PwGraph {
            streams: vec![node("High Tide")],
            ..Default::default()
        };
        let media = StreamInfo {
            app: Some("High Tide".into()),
            sr: Some(48000),
            source_processing_checked: true,
            ..Default::default()
        };
        let chosen = select_stream(&graph, None, Some(media)).unwrap();
        assert!(chosen.source_processing_checked);
        assert!(chosen.source_processing.is_none());
    }

    #[test]
    fn generic_python_merges_single_playing_mpris_player() {
        let graph = PwGraph {
            streams: vec![node("python3.13")],
            ..Default::default()
        };
        let player = mpris::choose_mpris_player(
            Some("python3.13"),
            vec![StreamInfo {
                app: Some("High Tide".into()),
                track: Some("Track".into()),
                ..Default::default()
            }],
        );
        let chosen = select_stream(&graph, None, player).unwrap();
        assert_eq!(chosen.app.as_deref(), Some("High Tide"));
        assert_eq!(chosen.track.as_deref(), Some("Track"));
        assert!(chosen.codec.is_none());
        assert!(chosen.sr.is_none());
    }

    #[test]
    fn unrelated_player_cannot_contribute_anonymous_audio_metadata() {
        let graph = PwGraph {
            streams: vec![StreamNode {
                id: 4,
                app_name: "Firefox".into(),
                node_name: "Firefox".into(),
                media_name: "Playing".into(),
                pid: None,
            }],
            ..Default::default()
        };
        let selected = select_stream(
            &graph,
            None,
            Some(StreamInfo {
                app: Some("Instance_1_50086".into()),
                codec: Some("FLAC".into()),
                sr: Some(192000),
                bps: Some(24),
                track: Some("Song".into()),
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(selected.codec.is_none());
        assert_eq!(selected.sr, None);
        assert_eq!(selected.track.as_deref(), Some("Song"));
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
