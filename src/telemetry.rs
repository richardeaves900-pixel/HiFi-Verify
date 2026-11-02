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
    /// Unique direct playback routes observed in the live PipeWire graph.
    pub playback_routes: Vec<PlaybackRoute>,
}

// Recognise stable Flatpak identities before generic Python process names.
// Do not infer a codec or a TIDAL source sample rate from an application ID.
fn known_application_id(props: &serde_json::Value) -> Option<&'static str> {
    ["pipewire.access.portal.app_id", "application.id"]
        .iter()
        .filter_map(|key| props.get(*key).and_then(|v| v.as_str()))
        .find_map(|id| match id.to_ascii_lowercase().as_str() {
            "io.github.nokse22.high-tide" | "io.github.nokse22.hightide" => Some("High Tide"),
            _ => None,
        })
}

fn uninformative_app_name(name: &str) -> bool {
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

fn process_pid(props: &serde_json::Value) -> Option<i32> {
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

fn resolve_playback_app(
    node_props: &serde_json::Value,
    client_props: Option<&serde_json::Value>,
    pid: Option<i32>,
) -> String {
    // Known app identifiers outrank generic process names on either object.
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
    // /proc comm is a last resort, AFTER all PipeWire node/client metadata.
    let fallback = playback_app_name(&serde_json::Value::Null, pid);
    if !uninformative_app_name(&fallback) {
        fallback
    } else {
        node
    }
}

// Identify the *producer* of a running playback stream. The /proc comm field
// is a last resort: Firefox subprocess names may describe a content process
// rather than the Firefox application, and sandboxed PIDs may be inaccessible.
fn playback_app_name(props: &serde_json::Value, pid: Option<i32>) -> String {
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

fn is_dsp_name(name: &str) -> bool {
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

fn is_playback_candidate(n: &StreamNode) -> bool {
    !is_dsp_name(&format!("{} {}", n.app_name, n.node_name))
}

fn is_mpd_app(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "mpd" | "music player daemon"
    )
}

fn pw_rate(v: &serde_json::Value) -> Option<u32> {
    let rate = v.as_u64().and_then(|n| u32::try_from(n).ok()).or_else(|| {
        v.as_str().and_then(|s| {
            // node.rate is a SPA fraction such as 1/96000 (seconds/sample).
            if let Some((numer, denom)) = s.split_once('/') {
                (numer == "1").then(|| denom.parse::<u32>().ok()).flatten()
            } else {
                s.parse::<u32>().ok()
            }
        })
    });
    rate.filter(|r| (8000..=768000).contains(r))
}

fn node_negotiated_rate(props: &serde_json::Value, info: &serde_json::Value) -> Option<u32> {
    props
        .get("audio.rate")
        .and_then(pw_rate)
        .or_else(|| {
            info.get("params")
                .and_then(|p| p.get("Format"))
                .and_then(|format| {
                    format.get("rate").and_then(pw_rate).or_else(|| {
                        format.as_array().and_then(|items| {
                            items.iter().find_map(|f| f.get("rate").and_then(pw_rate))
                        })
                    })
                })
        })
        .or_else(|| props.get("node.rate").and_then(pw_rate))
}

/// A measured running ALSA output clock, never an arbitrary pw-top node.
/// When several outputs are active, do not attribute one without a route.
pub fn observed_output_clock(rows: &[crate::pwtop::NodeRow]) -> (Option<u32>, Option<u32>) {
    let mut outputs = rows
        .iter()
        .filter(|r| r.status == 'R' && r.name.starts_with("alsa_output."));
    let Some(output) = outputs.next() else {
        return (None, None);
    };
    if outputs.next().is_some() {
        return (None, None);
    }
    (
        output.rate.parse::<u32>().ok().filter(|n| *n > 0),
        output.quant.parse::<u32>().ok().filter(|n| *n > 0),
    )
}

#[cfg(test)]
mod observer_output_tests {
    use super::*;
    fn node(name: &str, status: char, rate: &str) -> crate::pwtop::NodeRow {
        crate::pwtop::NodeRow {
            wait_us: None,
            status,
            id: "1".into(),
            quant: "512".into(),
            rate: rate.into(),
            wait: "0".into(),
            busy: "0".into(),
            wq: 0.0,
            bq: 0.0,
            err: 0,
            fmt: "-".into(),
            name: name.into(),
        }
    }
    #[test]
    fn correct_dac_rate_overrides_unrelated_48k_stream() {
        let rows = [
            node("alsa_output.usb-FiiO", 'R', "96000"),
            node("python3.13", 'R', "48000"),
        ];
        assert_eq!(observed_output_clock(&rows).0, Some(96000));
    }
    #[test]
    fn multiple_outputs_are_not_misattributed() {
        let rows = [
            node("alsa_output.usb-FiiO", 'R', "96000"),
            node("alsa_output.pci-HDMI", 'R', "48000"),
        ];
        assert_eq!(observed_output_clock(&rows), (None, None));
    }
    #[test]
    fn idle_output_is_not_reported_as_live() {
        assert_eq!(
            observed_output_clock(&[node("alsa_output.usb-FiiO", 'I', "96000")]),
            (None, None)
        );
    }
}

/// Decoded *PipeWire* playback PCM, not a claim about the source file's codec.
/// Ambiguous concurrent producers are not guessed or combined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackPcm {
    pub format: String,
    pub rate: u32,
    pub channels: Option<u32>,
}

pub fn observed_playback_pcm(rows: &[crate::pwtop::NodeRow]) -> Option<PlaybackPcm> {
    let mut candidates = rows.iter().filter_map(|r| {
        if r.status != 'R'
            || r.name.starts_with("alsa_output.")
            || r.name.starts_with("alsa_input.")
        {
            return None;
        }
        let name = r.name.to_ascii_lowercase();
        if is_dsp_name(&name)
            || ["monitor", "dummy-driver", "freewheel-driver"]
                .iter()
                .any(|part| name.contains(part))
        {
            return None;
        }
        // pw-top already exposes the actual negotiated decoded PCM format.
        let format = r.fmt.split_whitespace().next()?.to_string();
        if !(format.starts_with('S') || format.starts_with('F')) {
            return None;
        }
        let rate: u32 = r.rate.parse().ok()?;
        if !(8000..=768000).contains(&rate) {
            return None;
        }
        let channels = r
            .fmt
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.strip_suffix('c'))
            .and_then(|s| s.parse().ok());
        Some(PlaybackPcm {
            format,
            rate,
            channels,
        })
    });
    let first = candidates.next()?;
    if candidates.next().is_some() {
        None
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod playback_pcm_observer_tests {
    use super::*;
    fn row(name: &str, fmt: &str, rate: &str) -> crate::pwtop::NodeRow {
        crate::pwtop::NodeRow {
            wait_us: None,
            status: 'R',
            id: "1".into(),
            quant: "512".into(),
            rate: rate.into(),
            wait: "0".into(),
            busy: "0".into(),
            wq: 0.0,
            bq: 0.0,
            err: 0,
            fmt: fmt.into(),
            name: name.into(),
        }
    }
    #[test]
    fn high_tide_pcm_is_measured_without_modified_player() {
        let rows = [
            row("alsa_output.usb-FiiO", "S32LE 2c", "96000"),
            row("python3.13", "S24LE 2c", "96000"),
            row("cava", "S16LE 2c", "48000"),
        ];
        assert_eq!(
            observed_playback_pcm(&rows),
            Some(PlaybackPcm {
                format: "S24LE".into(),
                rate: 96000,
                channels: Some(2)
            })
        );
    }
    #[test]
    fn concurrent_audio_producers_are_ambiguous() {
        let rows = [
            row("python3.13", "S24LE 2c", "96000"),
            row("firefox", "F32LE 2c", "48000"),
        ];
        assert_eq!(observed_playback_pcm(&rows), None);
    }
    #[test]
    fn output_is_not_mistaken_for_source_stream() {
        assert_eq!(
            observed_playback_pcm(&[row("alsa_output.usb-FiiO", "S32LE 2c", "96000")]),
            None
        );
    }
}

/// One stereo playback route appears as two Port links; the node pair is
/// deduplicated before making a verdict. Monitoring/visualiser connections are
/// not playback routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRoute {
    pub stream_id: u64,
    pub sink_id: u64,
    pub sink_name: String,
    pub alsa_card: Option<u32>,
}

fn json_id(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse::<u64>().ok())
}

fn playback_routes_from_dump(objects: &[serde_json::Value]) -> Vec<PlaybackRoute> {
    use std::collections::HashMap;
    let nodes: HashMap<u64, &serde_json::Value> = objects
        .iter()
        .filter(|o| {
            o.get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.ends_with(":Node"))
        })
        .filter_map(|o| Some((json_id(o.get("id")?)?, o)))
        .collect();
    let mut routes = Vec::new();
    for link in objects.iter().filter(|o| {
        o.get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.ends_with(":Link"))
    }) {
        let Some(info) = link.get("info") else {
            continue;
        };
        if info.get("state").and_then(|v| v.as_str()) != Some("active") {
            continue;
        }
        let Some(source_id) = info.get("output-node-id").and_then(json_id) else {
            continue;
        };
        let Some(sink_id) = info.get("input-node-id").and_then(json_id) else {
            continue;
        };
        let Some(source) = nodes.get(&source_id) else {
            continue;
        };
        let Some(sink) = nodes.get(&sink_id) else {
            continue;
        };
        let src_props = &source["info"]["props"];
        let sink_props = &sink["info"]["props"];
        if src_props["media.class"].as_str() != Some("Stream/Output/Audio")
            || sink_props["media.class"].as_str() != Some("Audio/Sink")
            || source["info"]["state"].as_str() != Some("running")
        {
            continue;
        }
        let Some(sink_name) = sink_props["node.name"].as_str() else {
            continue;
        };
        if !sink_name.starts_with("alsa_output.") {
            continue;
        }
        let card = ["api.alsa.card", "api.alsa.pcm.card", "alsa.card"]
            .iter()
            .find_map(|key| sink_props.get(*key).and_then(json_id))
            .and_then(|id| u32::try_from(id).ok());
        let route = PlaybackRoute {
            stream_id: source_id,
            sink_id,
            sink_name: sink_name.to_string(),
            alsa_card: card,
        };
        if !routes
            .iter()
            .any(|r: &PlaybackRoute| r.stream_id == source_id && r.sink_id == sink_id)
        {
            routes.push(route);
        }
    }
    routes
}

/// The original codec is never inferred from the PCM transport. Green denotes
/// a clean *observed* rate/format path, not identical decoded source samples.
#[derive(Debug, Clone)]
pub struct PcmPathAssessment {
    pub status: &'static str, // idle | clear | altered | unknown
    pub explanation: String,
    pub pcm: Option<PlaybackPcm>,
    pub output_rate: Option<u32>,
    pub output_quantum: Option<u32>,
}

fn card_from_hw_path(path: &str) -> Option<u32> {
    path.strip_prefix("/proc/asound/card")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

impl PwGraph {
    pub fn unique_playback_route(&self) -> Option<&PlaybackRoute> {
        if self.playback_routes.len() == 1 {
            self.playback_routes.first()
        } else {
            None
        }
    }

    pub fn hardware_for_route<'a>(
        &self,
        hw: &'a [crate::model::HwParams],
    ) -> Option<&'a crate::model::HwParams> {
        let route = self.unique_playback_route()?;
        if let Some(card) = route.alsa_card {
            let mut matched = hw
                .iter()
                .filter(|h| card_from_hw_path(&h.path) == Some(card));
            let first = matched.next()?;
            if matched.next().is_none() {
                Some(first)
            } else {
                None
            }
        } else if hw.len() == 1 {
            hw.first()
        } else {
            None
        }
    }

    pub fn pcm_for_route(&self, rows: &[crate::pwtop::NodeRow]) -> Option<PlaybackPcm> {
        let route = self.unique_playback_route()?;
        let node = rows
            .iter()
            .find(|r| r.status == 'R' && r.id.parse::<u64>().ok() == Some(route.stream_id))?;
        let format = node.fmt.split_whitespace().next()?.to_string();
        if !format.starts_with('S') && !format.starts_with('U') && !format.starts_with('F') {
            return None;
        }
        let rate = node
            .rate
            .parse::<u32>()
            .ok()
            .filter(|n| (8000..=768000).contains(n))?;
        let channels = node
            .fmt
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.strip_suffix('c'))
            .and_then(|s| s.parse().ok());
        Some(PlaybackPcm {
            format,
            rate,
            channels,
        })
    }

    pub fn assess_pcm_path(
        &self,
        rows: &[crate::pwtop::NodeRow],
        hw: &[crate::model::HwParams],
        stream: &StreamInfo,
    ) -> PcmPathAssessment {
        let pcm = self.pcm_for_route(rows);
        let endpoint = self.hardware_for_route(hw);
        let output_node = self.unique_playback_route().and_then(|route| {
            rows.iter()
                .find(|r| r.status == 'R' && r.id.parse::<u64>().ok() == Some(route.sink_id))
        });
        let output_rate = endpoint
            .map(|h| h.rate)
            .or_else(|| output_node.and_then(|r| r.rate.parse::<u32>().ok()));
        let output_quantum = output_node.and_then(|r| r.quant.parse::<u32>().ok());
        let (status, explanation) = if stream.app.is_none() {
            ("idle", "No active playback".to_string())
        } else if stream.source_processing.is_some() {
            (
                "altered",
                "Processing reported on the audio path".to_string(),
            )
        } else if let (Some(p), Some(h)) = (pcm.as_ref(), endpoint) {
            if p.rate != h.rate {
                (
                    "altered",
                    format!(
                        "RATE CONVERSION: {:.1} -> {:.1} kHz",
                        p.rate as f64 / 1000.0,
                        h.rate as f64 / 1000.0
                    ),
                )
            } else if stream.sr.is_some_and(|src| src != p.rate) {
                ("altered", "Source and decoded PCM rates differ".to_string())
            } else if p.channels.is_some() && h.channels.is_some() && p.channels != h.channels {
                (
                    "altered",
                    "Channel count changes between stream and DAC".to_string(),
                )
            } else if p.channels.is_none() || h.channels.is_none() {
                (
                    "unknown",
                    "Playback or DAC channel count not measured".to_string(),
                )
            } else if p.format.starts_with('F') || p.format.starts_with('U') {
                (
                    "unknown",
                    "PCM sample conversion cannot be ruled out".to_string(),
                )
            } else if p.format.starts_with('S')
                && p.format[1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u8>()
                    .ok()
                    .zip(h.bits)
                    .is_some_and(|(a, b)| a > b)
            {
                (
                    "altered",
                    "DAC format has fewer bits than stream PCM".to_string(),
                )
            } else if p.format.starts_with('S') && h.bits.is_some() {
                (
                    "clear",
                    "PCM PATH CLEAR (SAMPLE IDENTITY UNVERIFIED)".to_string(),
                )
            } else {
                (
                    "unknown",
                    "PCM precision or negotiated format not established".to_string(),
                )
            }
        } else {
            (
                "unknown",
                "Playback route or negotiated PCM not measured".to_string(),
            )
        };
        PcmPathAssessment {
            status,
            explanation,
            pcm,
            output_rate,
            output_quantum,
        }
    }
}

#[cfg(test)]
mod linked_path_tests {
    use super::*;
    fn graph_fixture() -> PwGraph {
        let dump = serde_json::json!([
            {"id":12,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Stream/Output/Audio","node.name":"python3.13"}}},
            {"id":44,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Audio/Sink","node.name":"alsa_output.usb-Fiio","api.alsa.card":"3"}}},
            {"id":52,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Stream/Input/Audio","node.name":"cava"}}},
            {"id":13,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":12,"input-node-id":44}},
            {"id":14,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":12,"input-node-id":44}},
            {"id":15,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":44,"input-node-id":52}}
        ]);
        PwGraph {
            playback_routes: playback_routes_from_dump(dump.as_array().unwrap()),
            ..Default::default()
        }
    }
    fn node(id: &str, format: &str, rate: &str, name: &str) -> crate::pwtop::NodeRow {
        crate::pwtop::NodeRow {
            wait_us: None,
            status: 'R',
            id: id.into(),
            quant: "512".into(),
            rate: rate.into(),
            wait: "0".into(),
            busy: "0".into(),
            wq: 0.0,
            bq: 0.0,
            err: 0,
            fmt: format.into(),
            name: name.into(),
        }
    }
    #[test]
    fn direct_high_tide_route_ignores_cava_and_duplicate_channel_links() {
        let graph = graph_fixture();
        assert_eq!(graph.playback_routes.len(), 1);
        let pcm = graph
            .pcm_for_route(&[
                node("12", "S24LE 2c", "96000", "python3.13"),
                node("44", "S32LE 2c", "48000", "alsa_output.usb-Fiio"),
                node("52", "S16LE 2c", "48000", "cava"),
            ])
            .unwrap();
        assert_eq!(pcm.rate, 96000);
        assert_eq!(pcm.format, "S24LE");
    }
    #[test]
    fn mismatch_is_red_and_matching_widened_pcm_is_green() {
        let g = graph_fixture();
        let source = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        let mut hw = vec![crate::model::HwParams {
            path: "/proc/asound/card3/pcm0p/sub0/hw_params".into(),
            format: "S32_LE".into(),
            rate: 48000,
            channels: Some(2),
            bits: Some(32),
            ..Default::default()
        }];
        let input = [
            node("12", "S24LE 2c", "96000", "python3.13"),
            node("44", "S32LE 2c", "48000", "alsa_output.usb-Fiio"),
        ];
        assert_eq!(g.assess_pcm_path(&input, &hw, &source).status, "altered");
        hw[0].rate = 96000;
        assert_eq!(g.assess_pcm_path(&input, &hw, &source).status, "clear");
    }
    #[test]
    fn missing_link_never_produces_false_green() {
        let g = PwGraph::default();
        let source = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        assert_eq!(g.assess_pcm_path(&[], &[], &source).status, "unknown");
    }
}

impl PwGraph {
    pub fn load() -> Self {
        let Ok(out) = crate::host::host_cmd(&["pw-dump"], Duration::from_secs(3)) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
            return Self::default();
        };
        let Some(objects) = value.as_array() else {
            return Self::default();
        };
        Self::from_dump(objects)
    }

    fn from_dump(objects: &[serde_json::Value]) -> Self {
        use std::collections::{HashMap, HashSet};
        let mut g = Self::default();
        let client_props: HashMap<u64, &serde_json::Value> = objects
            .iter()
            .filter(|obj| {
                obj.get("type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.ends_with(":Client"))
            })
            .filter_map(|obj| Some((json_id(obj.get("id")?)?, obj.get("info")?.get("props")?)))
            .collect();
        let mut seen_processing = HashSet::new();
        let mut hardware_clocks: Vec<(u64, Option<u32>, Option<u32>)> = Vec::new();

        for obj in objects {
            let Some(info) = obj.get("info") else {
                continue;
            };
            let Some(props) = info.get("props") else {
                continue;
            };
            let media_class = props
                .get("media.class")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let state = info.get("state").and_then(|v| v.as_str()).unwrap_or("");
            let oid = obj.get("id").and_then(json_id).unwrap_or(0);
            let app = props
                .get("application.name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let node = props
                .get("node.name")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if state == "running" && media_class == "Audio/Sink" && node.starts_with("alsa_output.")
            {
                let rate = node_negotiated_rate(props, info);
                let quantum = props
                    .get("audio.quantum")
                    .and_then(json_id)
                    .and_then(|n| u32::try_from(n).ok())
                    .or_else(|| {
                        props
                            .get("node.quantum")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.split('/').next())
                            .and_then(|s| s.parse::<u32>().ok())
                    });
                hardware_clocks.push((oid, rate, quantum));
            }

            if media_class == "Audio/Sink" {
                let desc = props
                    .get("node.description")
                    .and_then(|v| v.as_str())
                    .or_else(|| props.get("node.nick").and_then(|v| v.as_str()))
                    .unwrap_or(node);
                g.sinks.push(Sink {
                    id: oid.to_string(),
                    description: format!("{} (ID:{})", desc, oid),
                });
            }

            if media_class == "Stream/Output/Audio" && state == "running" {
                let client_id = props
                    .get("client.id")
                    .and_then(json_id)
                    .or_else(|| info.get("client-id").and_then(json_id));
                let owner = client_id.and_then(|id| client_props.get(&id).copied());
                let pid = process_pid(props).or_else(|| owner.and_then(process_pid));
                g.streams.push(StreamNode {
                    id: oid,
                    app_name: resolve_playback_app(props, owner, pid),
                    node_name: node.to_string(),
                    media_name: props
                        .get("media.name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    pid,
                });
            }

            if state == "running" && is_dsp_name(&format!("{} {}", app, node)) {
                let lower = format!("{} {}", app, node).to_ascii_lowercase();
                let name = if lower.contains("easyeffects") || lower.contains("ee_") {
                    "EasyEffects".to_string()
                } else if lower.contains("jamesdsp") {
                    "JamesDSP".to_string()
                } else if lower.contains("carla") {
                    "Carla".to_string()
                } else if lower.contains("calf") {
                    "Calf".to_string()
                } else if lower.contains("lsp-") || lower.contains("lsp_") {
                    "LSP Plugins".to_string()
                } else if lower.contains("cava") || lower.contains("hifi-verify") {
                    // A monitor is not necessarily inserted in the playback path.
                    continue;
                } else if !app.is_empty() {
                    app.to_string()
                } else {
                    node.to_string()
                };
                if seen_processing.insert(name.clone()) {
                    g.processing.push(name);
                }
            }
        }
        g.playback_routes = playback_routes_from_dump(objects);
        // Only use a measured hardware output, preferably the connected one.
        let active = g
            .unique_playback_route()
            .and_then(|r| hardware_clocks.iter().find(|(id, _, _)| *id == r.sink_id))
            .or_else(|| {
                if hardware_clocks.len() == 1 {
                    hardware_clocks.first()
                } else {
                    None
                }
            });
        if let Some((_, rate, quantum)) = active {
            g.driver_rate = *rate;
            g.driver_quantum = *quantum;
        }
        g
    }

    pub fn primary_stream(&self) -> Option<StreamInfo> {
        // Visualisers and DSP helper nodes are not the audio-producing app.
        let first_valid = self
            .unique_playback_route()
            .and_then(|route| {
                self.streams
                    .iter()
                    .find(|n| n.id == route.stream_id && is_playback_candidate(n))
            })
            .or_else(|| self.streams.iter().find(|n| is_playback_candidate(n)));

        first_valid.map(|n| {
            // Names and stream producers do not establish the source codec.
            // A real accessible media file may establish its container format.

            StreamInfo {
                codec: n.pid.and_then(sniff_pid_files),
                track: Some(if n.media_name.is_empty() {
                    n.app_name.clone()
                } else {
                    n.media_name.clone()
                }),
                app: Some(n.app_name.clone()),
                sr: None,
                bps: None,
                br: None,
                source_processing: None,
                source_processing_checked: false,
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
            // A Flatpak can publish only "python3.13" as its client name.
            // If precisely one audio stream is running and MPRIS reports a
            // specific playing player, use that name for attribution only.
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
                    // Anonymous players can enrich a title, never source properties.
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

    // No PipeWire playback node seen: preserve the existing MPD/MPRIS fallback.
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

fn parse_mpd_audio(audio: &str) -> (Option<u32>, Option<u8>) {
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

fn variant_u32(value: &glib::Variant) -> Option<u32> {
    value
        .get::<u32>()
        .or_else(|| value.get::<u64>().and_then(|v| u32::try_from(v).ok()))
        .or_else(|| value.get::<i32>().and_then(|v| u32::try_from(v).ok()))
        .or_else(|| value.get::<i64>().and_then(|v| u32::try_from(v).ok()))
}

fn local_flac_path(url: &str) -> Option<std::path::PathBuf> {
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
            ) && !lower.starts_with("instance")
                && !lower.chars().all(|c| c.is_ascii_digit())
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

// The High Tide bridge publishes raw GStreamer playbin volume and the
// normalization toggle. A missing key is UNKNOWN, not disabled. Even two
// clean reports do not prove sample identity or unity gain downstream.
fn high_tide_gain_report(
    normalized: Option<bool>,
    volume: Option<f64>,
    raw_gain_bridge: bool,
) -> (bool, Option<String>) {
    let volume = volume.filter(|v| v.is_finite() && (0.0..=10.0).contains(v));
    // The previous bridge rounded volume: a value of 1.0 could mean 0.96.
    // Only bridge protocol v2 (raw playbin gain) can establish "reported clear".
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

pub fn mpris_query_for(preferred_app: Option<&str>) -> Option<StreamInfo> {
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

    let mut candidates: Vec<StreamInfo> = Vec::new();
    for name in names
        .iter()
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
    {
        let identity = friendly_app_name(name);
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
        let mut source_processing: Option<String> = None;
        let mut source_processing_checked = false;
        // The application name never establishes a codec or sample rate.

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
            // Optional metadata added to High Tide's MPRIS implementation.
            // Values describe the actual TIDAL source manifest, not PCM output.
            if identity == "High Tide" {
                const P: &str = "io.github.nokse22.high-tide:";
                codec = meta
                    .get(&format!("{P}codec"))
                    .and_then(|v| v.get::<String>())
                    .filter(|s| !s.trim().is_empty());
                sr = meta
                    .get(&format!("{P}sample-rate"))
                    .and_then(variant_u32)
                    .filter(|v| (8000..=768000).contains(v));
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
                track = t;
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
            app: Some(friendly_app_name(name)),
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
fn choose_mpris_player(preferred: Option<&str>, candidates: Vec<StreamInfo>) -> Option<StreamInfo> {
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
        // Generic browser MPRIS names may be associated with one unambiguous
        // producer, but must never provide guessed source audio properties.
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

fn mpd_config_issues(config: &str, path: &str) -> Vec<String> {
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
        let keys: std::collections::HashMap<&str, &str> = block
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
        issues.extend(mpd_config_issues(&text, p));
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
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    let Some(reply_type) = glib::VariantTy::new("(as)").ok() else {
        return;
    };
    let Ok(reply) = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        None,
        Some(reply_type),
        gio::DBusCallFlags::NONE,
        250,
        gio::Cancellable::NONE,
    ) else {
        return;
    };
    let Some((names,)) = reply.get::<(Vec<String>,)>() else {
        return;
    };
    let mut available = Vec::new();
    let mut playing = Vec::new();
    for name in names
        .iter()
        .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
    {
        available.push(name.as_str());
        let Ok(reply) = bus.call_sync(
            Some(name.as_str()),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&glib::Variant::from(("org.mpris.MediaPlayer2.Player",))),
            glib::VariantTy::new("(a{sv})").ok(),
            gio::DBusCallFlags::NONE,
            250,
            gio::Cancellable::NONE,
        ) else {
            continue;
        };
        let Some((properties,)) =
            reply.get::<(std::collections::HashMap<String, glib::Variant>,)>()
        else {
            continue;
        };
        if properties
            .get("PlaybackStatus")
            .and_then(|v| v.get::<String>())
            .as_deref()
            == Some("Playing")
        {
            playing.push(name.as_str());
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
mod app_attribution_tests {
    use super::*;

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
    fn high_tide_flatpak_id_overrides_generic_python() {
        let props = serde_json::json!({
            "application.name": "python3.13",
            "pipewire.access.portal.app_id": "io.github.nokse22.high-tide",
        });
        assert_eq!(playback_app_name(&props, None), "High Tide");
    }

    #[test]
    fn high_tide_client_identity_is_used_when_node_name_is_missing() {
        let node = serde_json::json!({ "node.name": "python3.13" });
        let client = serde_json::json!({
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
        assert_eq!(chosen.sr, None, "must not invent source sample rate");
        assert_eq!(chosen.codec, None, "must not infer FLAC from app name");
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
        // Invalid values must not be treated as confirmation of clean gain.
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
        let player = choose_mpris_player(
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
    fn no_media_sources_reports_idle() {
        let graph = PwGraph {
            streams: vec![node("cava")],
            ..Default::default()
        };
        assert!(select_stream(&graph, None, None).is_none());
    }
}

#[cfg(test)]
mod repaired_regressions {
    use super::*;
    use glib::prelude::ToVariant;
    use serde_json::json;

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
    fn normalizes_mpd_identity_and_audio_formats() {
        assert_eq!(
            playback_app_name(&json!({"application.name":"Music Player Daemon"}), None),
            "MPD"
        );
        assert_eq!(parse_mpd_audio("48000:f:2"), (Some(48000), None));
        assert_eq!(parse_mpd_audio("dsd64:2"), (Some(2822400), Some(1)));
    }
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
    fn file_uri_is_percent_decoded_and_remote_uri_is_rejected() {
        let path = local_flac_path("file:///home/user/Music/01%20Track%20%C3%A9.flac").unwrap();
        assert_eq!(path.to_string_lossy(), "/home/user/Music/01 Track é.flac");
        assert!(local_flac_path("https://example.com/stream.flac").is_none());
    }
    #[test]
    fn mpd_comments_do_not_create_fake_settings() {
        let cfg = "# audio_output {\n# device \"default\"\n# }\naudio_output {\n type \"alsa\"\n device \"hw:0,0\"\n # mixer_type \"hardware\"\n mixer_type \"none\"\n}\n";
        assert!(mpd_config_issues(cfg, "test").is_empty());
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
    fn client_only_pid_is_preserved_and_microphone_does_not_override_output() {
        let objects = json!([
            {"id":1,"type":"PipeWire:Interface:Client","info":{"props":{
                "application.name":"High Tide", "pipewire.sec.pid":"12345"}}},
            {"id":11,"type":"PipeWire:Interface:Node","info":{"state":"running", "props":{
                "media.class":"Stream/Output/Audio","client.id":1,"node.name":"python3.13"}}},
            {"id":20,"type":"PipeWire:Interface:Node","info":{"state":"running", "props":{
                "media.class":"Audio/Source","node.name":"alsa_input.mic","audio.rate":48000}}},
            {"id":30,"type":"PipeWire:Interface:Node","info":{"state":"running", "props":{
                "media.class":"Audio/Sink","node.name":"alsa_output.usb-Fiio","node.rate":"1/96000"}}},
            {"id":42,"type":"PipeWire:Interface:Link","info":{
                "state":"active","output-node-id":11,"input-node-id":30}}
        ]);
        let graph = PwGraph::from_dump(objects.as_array().unwrap());
        assert_eq!(graph.streams[0].pid, Some(12345));
        assert_eq!(graph.streams[0].app_name, "High Tide");
        assert_eq!(graph.driver_rate, Some(96000));
    }
    #[test]
    fn repeated_easyeffects_nodes_count_once() {
        let objects = json!([
            {"id":1,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"ee_sie_1", "application.name":"EasyEffects"}}},
            {"id":2,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"ee_sie_2", "application.name":"EasyEffects"}}}
        ]);
        let graph = PwGraph::from_dump(objects.as_array().unwrap());
        assert_eq!(graph.processing, vec!["EasyEffects"]);
        assert!(graph.primary_stream().is_none());
    }
    #[test]
    fn accepts_full_gstreamer_playbin_volume_range() {
        let (known, issue) = high_tide_gain_report(Some(false), Some(3.5), true);
        assert!(known);
        assert!(issue.unwrap().contains("3.5000"));
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
    #[test]
    fn unrelated_dsp_does_not_falsely_red_flag_a_direct_hardware_route() {
        let objects = json!([
            {"id":11,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"python3.13"}}},
            {"id":30,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Audio/Sink","node.name":"alsa_output.usb-Fiio"}}},
            {"id":80,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"ee_sie_filter","application.name":"EasyEffects"}}},
            {"id":42,"type":"PipeWire:Interface:Link","info":{
                "state":"active","output-node-id":11,"input-node-id":30}}
        ]);
        let graph = PwGraph::from_dump(objects.as_array().unwrap());
        let hw = [crate::model::HwParams {
            path: "/proc/asound/card3/pcm0p/sub0/hw_params".into(),
            format: "S32_LE".into(),
            rate: 96000,
            channels: Some(2),
            bits: Some(32),
            ..Default::default()
        }];
        let rows = [crate::pwtop::NodeRow {
            wait_us: None,
            status: 'R',
            id: "11".into(),
            quant: "512".into(),
            rate: "96000".into(),
            wait: "0".into(),
            busy: "0".into(),
            wq: 0.0,
            bq: 0.0,
            err: 0,
            fmt: "S24LE 2c".into(),
            name: "python3.13".into(),
        }];
        let stream = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        // No ALSA card id on the fixture sink, but only one active HW device.
        assert_eq!(graph.assess_pcm_path(&rows, &hw, &stream).status, "clear");
    }
}
