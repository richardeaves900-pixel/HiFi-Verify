use super::proc_sniff::{flac_header, local_flac_path};
use super::{generic_player_name, same_app, uninformative_app_name, StreamInfo};
use std::collections::HashMap;

pub(crate) fn variant_u32(value: &glib::Variant) -> Option<u32> {
    value
        .get::<u32>()
        .or_else(|| value.get::<u64>().and_then(|v| u32::try_from(v).ok()))
        .or_else(|| value.get::<i32>().and_then(|v| u32::try_from(v).ok()))
        .or_else(|| value.get::<i64>().and_then(|v| u32::try_from(v).ok()))
}

pub(crate) fn friendly_app_name(bus_name: &str) -> String {
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
        .rfind(|p| {
            let lower = p.to_lowercase();
            !matches!(
                lower.as_str(),
                "io" | "com" | "org" | "net" | "github" | "gitlab" | "instance"
            ) && !lower.starts_with("instance")
                && !lower.chars().all(|c| c.is_ascii_digit())
        })
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => "Player".into(),
            }
        })
        .unwrap_or_else(|| "Player".to_string())
}

// The High Tide bridge publishes raw GStreamer playbin volume and the
// normalization toggle. A missing key is UNKNOWN, not disabled. Even two
// clean reports do not prove sample identity or unity gain downstream.
pub(crate) fn high_tide_gain_report(
    normalized: Option<bool>,
    volume: Option<f64>,
    raw_gain_bridge: bool,
) -> (bool, Option<String>) {
    let volume = volume.filter(|v| v.is_finite() && (0.0..=10.0).contains(v));
    let checked = raw_gain_bridge && normalized.is_some() && volume.is_some();
    let mut reasons = Vec::new();
    if normalized == Some(true) {
        reasons.push("High Tide normalization/ReplayGain enabled".to_string());
    }
    if let Some(v) = volume {
        if v != 1.0 {
            reasons.push(format!(
                "High Tide internal playback volume is {:.4} (unity = 1.0)",
                v
            ));
        }
    }
    (checked, (!reasons.is_empty()).then(|| reasons.join("; ")))
}

struct MprisBusPlayer {
    bus_name: String,
    properties: HashMap<String, glib::Variant>,
}

/// Shared D-Bus enumeration of all MPRIS players and their Player properties.
fn enumerate_mpris_players() -> Option<(gio::DBusConnection, Vec<MprisBusPlayer>)> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
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

    let mut players = Vec::new();
    let prop_ty = glib::VariantTy::new("(a{sv})").ok()?;
    for name in names
        .into_iter()
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
    {
        let Ok(props) = bus.call_sync(
            Some(name.as_str()),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&glib::Variant::from(("org.mpris.MediaPlayer2.Player",))),
            Some(prop_ty),
            gio::DBusCallFlags::NONE,
            250,
            gio::Cancellable::NONE,
        ) else {
            continue;
        };
        if let Some((properties,)) = props.get::<(HashMap<String, glib::Variant>,)>() {
            players.push(MprisBusPlayer {
                bus_name: name,
                properties,
            });
        }
    }
    Some((bus, players))
}

pub fn mpris_query_for(preferred_app: Option<&str>) -> Option<StreamInfo> {
    let (_, players) = enumerate_mpris_players()?;
    let mut candidates: Vec<StreamInfo> = Vec::new();

    for player in players {
        if player
            .properties
            .get("PlaybackStatus")
            .and_then(|v| v.get::<String>())
            .as_deref()
            != Some("Playing")
        {
            continue;
        }

        let identity = friendly_app_name(&player.bus_name);
        let mut codec: Option<String> = None;
        let mut sr: Option<u32> = None;
        let mut bps: Option<u8> = None;
        let mut source_processing: Option<String> = None;
        let mut source_processing_checked = false;

        let mut track = player
            .bus_name
            .trim_start_matches("org.mpris.MediaPlayer2.")
            .split('.')
            .next()
            .unwrap_or("Player")
            .to_string();

        if let Some(meta) = player
            .properties
            .get("Metadata")
            .and_then(|v| v.get::<HashMap<String, glib::Variant>>())
        {
            if identity == "High Tide" {
                const P: &str = "io.github.nokse22.high-tide:";
                codec = meta
                    .get(&format!("{P}codec"))
                    .and_then(|v| v.get::<String>())
                    .filter(|s| !s.trim().is_empty());
                sr = meta
                    .get(&format!("{P}sample-rate"))
                    .and_then(variant_u32)
                    .filter(|v| (8000..=768_000).contains(v));
                bps = meta
                    .get(&format!("{P}bit-depth"))
                    .and_then(variant_u32)
                    .filter(|v| (1..=64).contains(v))
                    .map(|v| v as u8);
                let normalized = meta
                    .get(&format!("{P}normalization-enabled"))
                    .and_then(|v| v.get::<bool>());
                let volume = meta
                    .get(&format!("{P}player-volume"))
                    .and_then(|v| v.get::<f64>());
                let raw_gain_bridge = meta
                    .get(&format!("{P}gain-report-version"))
                    .and_then(variant_u32)
                    == Some(2);
                (source_processing_checked, source_processing) =
                    high_tide_gain_report(normalized, volume, raw_gain_bridge);
            }
            if let Some(t) = meta.get("xesam:title").and_then(|v| v.get::<String>()) {
                if !t.trim().is_empty() {
                    track = t;
                }
            }
            if let Some(url) = meta.get("xesam:url").and_then(|v| v.get::<String>()) {
                if let Some(path) = local_flac_path(&url) {
                    codec = Some("FLAC".into());
                    if let Some((s, b, _)) = flac_header(path.to_string_lossy().as_ref()) {
                        sr = Some(s);
                        bps = Some(b);
                    }
                }
            }
        }

        candidates.push(StreamInfo {
            codec,
            track: Some(track),
            app: Some(identity),
            sr,
            bps,
            br: None,
            source_processing,
            source_processing_checked,
            mpd: false,
        });
    }

    choose_mpris_player(preferred_app, candidates)
}

// Only associate a generic process with metadata when exactly one MPRIS
// player is Playing. Multiple simultaneous players remain ambiguous.
pub(crate) fn choose_mpris_player(
    preferred: Option<&str>,
    candidates: Vec<StreamInfo>,
) -> Option<StreamInfo> {
    if let Some(name) = preferred.filter(|s| !uninformative_app_name(s)) {
        let mut matches = candidates
            .iter()
            .filter(|p| p.app.as_deref().is_some_and(|a| same_app(a, name)));
        if let Some(found) = matches.next() {
            if matches.next().is_none() {
                return Some(found.clone());
            }
            return None;
        }
        if candidates.len() == 1
            && candidates[0]
                .app
                .as_deref()
                .is_some_and(generic_player_name)
        {
            return candidates.into_iter().next();
        }
        return None;
    }
    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

pub fn send_mpris_command(command: &str) {
    let Some((bus, players)) = enumerate_mpris_players() else {
        return;
    };
    let mut available = Vec::new();
    let mut playing = Vec::new();
    for p in &players {
        available.push(p.bus_name.as_str());
        if p.properties
            .get("PlaybackStatus")
            .and_then(|v| v.get::<String>())
            .as_deref()
            == Some("Playing")
        {
            playing.push(p.bus_name.as_str());
        }
    }
    // Never send transport commands to an arbitrary unrelated player.
    let target = if playing.len() == 1 {
        Some(playing[0])
    } else if playing.is_empty() && available.len() == 1 {
        Some(available[0])
    } else {
        None
    };
    if let Some(player) = target {
        let _ = bus.call_sync(
            Some(player),
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
mod tests {
    use super::*;
    use glib::prelude::ToVariant;

    #[test]
    fn firefox_instance_is_not_the_player_name() {
        assert_eq!(
            friendly_app_name("org.mpris.MediaPlayer2.firefox.instance_1_50086"),
            "Firefox"
        );
    }

    #[test]
    fn variant_signed_integers_are_accepted_safely() {
        assert_eq!(variant_u32(&(-1_i32).to_variant()), None);
        assert_eq!(variant_u32(&(96000_i64).to_variant()), Some(96000));
    }

    #[test]
    fn high_tide_clean_requires_both_reports() {
        let (known, active) = high_tide_gain_report(Some(false), Some(1.0), true);
        assert!(known);
        assert!(active.is_none());
        let (known, active) = high_tide_gain_report(Some(false), None, true);
        assert!(!known);
        assert!(active.is_none());
        let (known, _) = high_tide_gain_report(None, Some(1.0), true);
        assert!(!known);
    }

    #[test]
    fn high_tide_nonunity_gain_is_not_silently_rounded_to_unity() {
        let (known, active) = high_tide_gain_report(Some(false), Some(0.96), true);
        assert!(known);
        assert!(active.unwrap().contains("0.9600"));
        let (known, active) = high_tide_gain_report(Some(false), Some(f64::NAN), true);
        assert!(!known);
        assert!(active.is_none());
    }

    #[test]
    fn old_rounded_volume_metadata_is_not_proof_of_unity() {
        let (known, active) = high_tide_gain_report(Some(false), Some(1.0), false);
        assert!(!known);
        assert!(active.is_none());
    }

    #[test]
    fn high_tide_enabled_normalization_warns_even_if_volume_missing() {
        let (known, active) = high_tide_gain_report(Some(true), None, true);
        assert!(!known);
        assert!(active.unwrap().contains("normalization"));
    }

    #[test]
    fn accepts_full_gstreamer_playbin_volume_range() {
        let (known, issue) = high_tide_gain_report(Some(false), Some(3.5), true);
        assert!(known);
        assert!(issue.unwrap().contains("3.5000"));
    }

    #[test]
    fn multiple_playing_players_do_not_rename_generic_python() {
        let candidates = vec![
            StreamInfo {
                app: Some("High Tide".into()),
                ..Default::default()
            },
            StreamInfo {
                app: Some("VLC".into()),
                ..Default::default()
            },
        ];
        assert!(choose_mpris_player(Some("python3.13"), candidates).is_none());
    }

    #[test]
    fn mpris_known_app_does_not_take_unrelated_player() {
        let candidates = vec![StreamInfo {
            app: Some("VLC".into()),
            ..Default::default()
        }];
        assert!(choose_mpris_player(Some("Firefox"), candidates).is_none());
    }

    #[test]
    fn unrelated_active_mpris_does_not_hide_preferred_player() {
        let selected = choose_mpris_player(
            Some("Firefox"),
            vec![
                StreamInfo {
                    app: Some("VLC".into()),
                    ..Default::default()
                },
                StreamInfo {
                    app: Some("Firefox".into()),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
        assert_eq!(selected.app.as_deref(), Some("Firefox"));
    }
}
