use super::proc_sniff::sniff_pid_files;
use super::{
    dsp_label, is_dsp_name, is_playback_candidate, process_pid, resolve_playback_app, StreamInfo,
    StreamNode,
};
use crate::model::{pcm_format_bits, HwParams, PathStatus, PlaybackPcm, Sink};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

/// A traced playback route from a running audio producer node to an ALSA sink.
/// Multi-hop routes through DSP/filter nodes are explicitly recorded so active
/// processing in the signal chain is flagged as `Altered` rather than `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRoute {
    pub stream_id: u64,
    pub sink_id: u64,
    pub sink_name: String,
    pub alsa_card: Option<u32>,
    pub dsp_in_route: Vec<String>,
    pub is_direct: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PwGraph {
    pub sinks: Vec<Sink>,
    pub streams: Vec<StreamNode>,
    pub processing: Vec<String>,
    pub driver_rate: Option<u32>,
    pub driver_quantum: Option<u32>,
    pub playback_routes: Vec<PlaybackRoute>,
}

/// The original codec is never inferred from the PCM transport. `Clear` denotes
/// a clean *observed* direct rate/format path, not identical decoded source samples.
#[derive(Debug, Clone)]
pub struct PcmPathAssessment {
    pub status: PathStatus,
    pub explanation: String,
    pub pcm: Option<PlaybackPcm>,
    pub output_rate: Option<u32>,
    pub output_quantum: Option<u32>,
    pub route_dsp: Vec<String>,
}

fn json_id(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse::<u64>().ok())
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
    rate.filter(|r| (8000..=768_000).contains(r))
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
    let rate = output
        .fmt_rate
        .or_else(|| output.rate.parse::<u32>().ok().filter(|n| *n > 0));
    let quant = output.quant.parse::<u32>().ok().filter(|n| *n > 0);
    (rate, quant)
}

fn card_from_hw_path(path: &str) -> Option<u32> {
    path.strip_prefix("/proc/asound/card")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

fn is_monitor_or_vu_node(props: &serde_json::Value) -> bool {
    let app = props
        .get("application.name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let node = props
        .get("node.name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lower = format!("{app} {node}").to_ascii_lowercase();
    lower.contains("cava") || lower.contains("hifi-verify")
}

/// Trace active PipeWire links from running playback producers to `alsa_output.*`
/// sinks (both direct 1-hop routes and multi-hop routes through DSP/filter nodes).
fn playback_routes_from_dump(objects: &[serde_json::Value]) -> Vec<PlaybackRoute> {
    let nodes: HashMap<u64, &serde_json::Value> = objects
        .iter()
        .filter(|o| {
            o.get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.ends_with(":Node"))
        })
        .filter_map(|o| Some((json_id(o.get("id")?)?, o)))
        .collect();

    // Build deduplicated directed adjacency list of active links.
    let mut adj: HashMap<u64, Vec<u64>> = HashMap::new();
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
        let Some(out_id) = info.get("output-node-id").and_then(json_id) else {
            continue;
        };
        let Some(in_id) = info.get("input-node-id").and_then(json_id) else {
            continue;
        };
        let entry = adj.entry(out_id).or_default();
        if !entry.contains(&in_id) {
            entry.push(in_id);
        }
    }

    // Collect running user playback producers (excluding DSP helper streams & monitors).
    let mut producer_ids: Vec<u64> = nodes
        .iter()
        .filter_map(|(&id, node)| {
            let info = node.get("info")?;
            if info.get("state").and_then(|v| v.as_str()) != Some("running") {
                return None;
            }
            let props = info.get("props")?;
            if props.get("media.class").and_then(|v| v.as_str()) != Some("Stream/Output/Audio") {
                return None;
            }
            let app = props
                .get("application.name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let nname = props
                .get("node.name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if is_dsp_name(&format!("{app} {nname}")) {
                return None;
            }
            Some(id)
        })
        .collect();
    producer_ids.sort_unstable();

    let mut routes = Vec::new();
    const MAX_HOPS: usize = 8;

    for start_id in producer_ids {
        // BFS from start_id to find reachable alsa_output.* sinks.
        let mut queue: VecDeque<(u64, usize, Vec<String>)> = VecDeque::new();
        let mut visited: HashSet<u64> = HashSet::new();
        visited.insert(start_id);
        queue.push_back((start_id, 0, Vec::new()));

        while let Some((curr_id, depth, dsp_so_far)) = queue.pop_front() {
            if depth >= MAX_HOPS {
                continue;
            }
            let Some(neighbors) = adj.get(&curr_id) else {
                continue;
            };
            for &next_id in neighbors {
                let Some(next_node) = nodes.get(&next_id) else {
                    continue;
                };
                let next_props = &next_node["info"]["props"];
                if is_monitor_or_vu_node(next_props) {
                    continue;
                }
                let media_class = next_props
                    .get("media.class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let node_name = next_props
                    .get("node.name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if media_class == "Audio/Sink" && node_name.starts_with("alsa_output.") {
                    let card = ["api.alsa.card", "api.alsa.pcm.card", "alsa.card"]
                        .iter()
                        .find_map(|key| next_props.get(*key).and_then(json_id))
                        .and_then(|id| u32::try_from(id).ok());
                    let is_direct = depth == 0 && dsp_so_far.is_empty();
                    if !routes
                        .iter()
                        .any(|r: &PlaybackRoute| r.stream_id == start_id && r.sink_id == next_id)
                    {
                        routes.push(PlaybackRoute {
                            stream_id: start_id,
                            sink_id: next_id,
                            sink_name: node_name.to_string(),
                            alsa_card: card,
                            dsp_in_route: dsp_so_far.clone(),
                            is_direct,
                        });
                    }
                    continue;
                }

                // Do not traverse into capture/input streams (monitors).
                if media_class == "Stream/Input/Audio" {
                    continue;
                }

                if visited.insert(next_id) {
                    let mut next_dsp = dsp_so_far.clone();
                    let app = next_props
                        .get("application.name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(label) = dsp_label(app, node_name) {
                        if !next_dsp.contains(&label) {
                            next_dsp.push(label);
                        }
                    } else if !node_name.is_empty() && !next_dsp.contains(&node_name.to_string()) {
                        next_dsp.push(node_name.to_string());
                    }
                    queue.push_back((next_id, depth + 1, next_dsp));
                }
            }
        }
    }

    routes
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

    pub fn from_dump(objects: &[serde_json::Value]) -> Self {
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

            if state == "running" {
                if let Some(name) = dsp_label(app, node) {
                    if seen_processing.insert(name.clone()) {
                        g.processing.push(name);
                    }
                }
            }
        }

        g.playback_routes = playback_routes_from_dump(objects);
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

    pub fn unique_playback_route(&self) -> Option<&PlaybackRoute> {
        if self.playback_routes.len() == 1 {
            self.playback_routes.first()
        } else {
            None
        }
    }

    pub fn hardware_for_route<'a>(&self, hw: &'a [HwParams]) -> Option<&'a HwParams> {
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
        // Prefer the stream's own negotiated FORMAT column rate (`fmt_rate`),
        // falling back to the graph RATE column when `fmt_rate` is unavailable.
        let rate = node
            .fmt_rate
            .or_else(|| node.rate.parse::<u32>().ok())
            .filter(|n| (8000..=768_000).contains(n))?;
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
        hw: &[HwParams],
        stream: &StreamInfo,
    ) -> PcmPathAssessment {
        let route = self.unique_playback_route();
        let route_dsp = route.map(|r| r.dsp_in_route.clone()).unwrap_or_default();
        let pcm = self.pcm_for_route(rows);
        let endpoint = self.hardware_for_route(hw);
        let output_node = route.and_then(|r| {
            rows.iter()
                .find(|row| row.status == 'R' && row.id.parse::<u64>().ok() == Some(r.sink_id))
        });
        let output_rate = endpoint.map(|h| h.rate).or_else(|| {
            output_node.and_then(|r| {
                r.fmt_rate
                    .or_else(|| r.rate.parse::<u32>().ok().filter(|v| *v > 0))
            })
        });
        let output_quantum =
            output_node.and_then(|r| r.quant.parse::<u32>().ok().filter(|q| *q > 0));

        let (status, explanation) = if stream.app.is_none() {
            (PathStatus::Idle, "No active playback".to_string())
        } else if let Some(ref src_proc) = stream.source_processing {
            (
                PathStatus::Altered,
                format!("Source processing active: {src_proc}"),
            )
        } else if let Some(r) = route.filter(|r| !r.is_direct) {
            let via = if r.dsp_in_route.is_empty() {
                "intermediate PipeWire node(s)".to_string()
            } else {
                r.dsp_in_route.join(" → ")
            };
            (
                PathStatus::Altered,
                format!("DSP / processing in playback route: {via}"),
            )
        } else if let (Some(p), Some(h)) = (pcm.as_ref(), endpoint) {
            if p.rate != h.rate {
                (
                    PathStatus::Altered,
                    format!(
                        "RATE CONVERSION: {:.1} → {:.1} kHz",
                        p.rate as f64 / 1000.0,
                        h.rate as f64 / 1000.0
                    ),
                )
            } else if stream.sr.is_some_and(|src| src != p.rate) {
                (
                    PathStatus::Altered,
                    "Source and decoded PCM rates differ".to_string(),
                )
            } else if p.channels.is_some() && h.channels.is_some() && p.channels != h.channels {
                (
                    PathStatus::Altered,
                    "Channel count changes between stream and DAC".to_string(),
                )
            } else if p.channels.is_none() || h.channels.is_none() {
                (
                    PathStatus::Unknown,
                    "Playback or DAC channel count not measured".to_string(),
                )
            } else if p.format.starts_with('F') || p.format.starts_with('U') {
                (
                    PathStatus::Unknown,
                    "Float/unsigned PCM conversion cannot be ruled out".to_string(),
                )
            } else if p.format.starts_with('S')
                && pcm_format_bits(&p.format)
                    .zip(h.bits)
                    .is_some_and(|(a, b)| a > b)
            {
                (
                    PathStatus::Altered,
                    "DAC format has fewer bits than stream PCM".to_string(),
                )
            } else if p.format.starts_with('S') && h.bits.is_some() {
                (
                    PathStatus::Clear,
                    "PCM path clear (sample identity unverified)".to_string(),
                )
            } else {
                (
                    PathStatus::Unknown,
                    "PCM precision or negotiated format not established".to_string(),
                )
            }
        } else {
            (
                PathStatus::Unknown,
                "Playback route or negotiated PCM not measured".to_string(),
            )
        };

        PcmPathAssessment {
            status,
            explanation,
            pcm,
            output_rate,
            output_quantum,
            route_dsp,
        }
    }

    pub fn primary_stream(&self) -> Option<StreamInfo> {
        let first_valid = self
            .unique_playback_route()
            .and_then(|route| {
                self.streams
                    .iter()
                    .find(|n| n.id == route.stream_id && is_playback_candidate(n))
            })
            .or_else(|| self.streams.iter().find(|n| is_playback_candidate(n)));

        first_valid.map(|n| StreamInfo {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node_row(
        id: &str,
        format: &str,
        rate: &str,
        fmt_rate: Option<u32>,
        name: &str,
    ) -> crate::pwtop::NodeRow {
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
            fmt_rate,
            name: name.into(),
        }
    }

    fn graph_fixture() -> PwGraph {
        let dump = json!([
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

    #[test]
    fn correct_dac_rate_overrides_unrelated_48k_stream() {
        let rows = [
            node_row(
                "44",
                "S32LE 2c",
                "96000",
                Some(96000),
                "alsa_output.usb-FiiO",
            ),
            node_row("12", "S24LE 2c", "48000", Some(48000), "python3.13"),
        ];
        assert_eq!(observed_output_clock(&rows).0, Some(96000));
    }

    #[test]
    fn multiple_outputs_are_not_misattributed() {
        let rows = [
            node_row(
                "44",
                "S32LE 2c",
                "96000",
                Some(96000),
                "alsa_output.usb-FiiO",
            ),
            node_row(
                "45",
                "S32LE 2c",
                "48000",
                Some(48000),
                "alsa_output.pci-HDMI",
            ),
        ];
        assert_eq!(observed_output_clock(&rows), (None, None));
    }

    #[test]
    fn idle_output_is_not_reported_as_live() {
        let mut idle = node_row(
            "44",
            "S32LE 2c",
            "96000",
            Some(96000),
            "alsa_output.usb-FiiO",
        );
        idle.status = 'I';
        assert_eq!(observed_output_clock(&[idle]), (None, None));
    }

    #[test]
    fn direct_high_tide_route_ignores_cava_and_duplicate_channel_links() {
        let graph = graph_fixture();
        assert_eq!(graph.playback_routes.len(), 1);
        assert!(graph.playback_routes[0].is_direct);
        let pcm = graph
            .pcm_for_route(&[
                node_row("12", "S24LE 2c", "0", Some(96000), "python3.13"),
                node_row(
                    "44",
                    "S32LE 2c",
                    "48000",
                    Some(48000),
                    "alsa_output.usb-Fiio",
                ),
                node_row("52", "S16LE 2c", "48000", Some(48000), "cava"),
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
        let mut hw = vec![HwParams {
            path: "/proc/asound/card3/pcm0p/sub0/hw_params".into(),
            format: "S32_LE".into(),
            rate: 48000,
            channels: Some(2),
            bits: Some(32),
            ..Default::default()
        }];
        let input = [
            node_row("12", "S24LE 2c", "96000", Some(96000), "python3.13"),
            node_row(
                "44",
                "S32LE 2c",
                "48000",
                Some(48000),
                "alsa_output.usb-Fiio",
            ),
        ];
        assert_eq!(
            g.assess_pcm_path(&input, &hw, &source).status,
            PathStatus::Altered
        );
        hw[0].rate = 96000;
        assert_eq!(
            g.assess_pcm_path(&input, &hw, &source).status,
            PathStatus::Clear
        );
    }

    #[test]
    fn missing_link_never_produces_false_green() {
        let g = PwGraph::default();
        let source = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        assert_eq!(
            g.assess_pcm_path(&[], &[], &source).status,
            PathStatus::Unknown
        );
    }

    #[test]
    fn multi_hop_easyeffects_route_is_flagged_as_altered() {
        let objects = json!([
            {"id":11,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"python3.13","application.name":"High Tide"}}},
            {"id":70,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Audio/Sink","node.name":"easyeffects_sink","application.name":"EasyEffects"}}},
            {"id":71,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Stream/Output/Audio","node.name":"ee_soe_output_level","application.name":"EasyEffects"}}},
            {"id":30,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{
                "media.class":"Audio/Sink","node.name":"alsa_output.usb-Fiio","api.alsa.card":"3"}}},
            {"id":100,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":11,"input-node-id":70}},
            {"id":101,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":70,"input-node-id":71}},
            {"id":102,"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":71,"input-node-id":30}}
        ]);
        let graph = PwGraph::from_dump(objects.as_array().unwrap());
        assert_eq!(graph.playback_routes.len(), 1);
        assert!(!graph.playback_routes[0].is_direct);
        assert_eq!(graph.playback_routes[0].dsp_in_route, vec!["EasyEffects"]);

        let hw = [HwParams {
            path: "/proc/asound/card3/pcm0p/sub0/hw_params".into(),
            format: "S32_LE".into(),
            rate: 96000,
            channels: Some(2),
            bits: Some(32),
            ..Default::default()
        }];
        let rows = [node_row(
            "11",
            "S24LE 2c",
            "96000",
            Some(96000),
            "python3.13",
        )];
        let stream = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        let assessment = graph.assess_pcm_path(&rows, &hw, &stream);
        assert_eq!(assessment.status, PathStatus::Altered);
        assert!(assessment.explanation.contains("EasyEffects"));
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
        let hw = [HwParams {
            path: "/proc/asound/card3/pcm0p/sub0/hw_params".into(),
            format: "S32_LE".into(),
            rate: 96000,
            channels: Some(2),
            bits: Some(32),
            ..Default::default()
        }];
        let rows = [node_row(
            "11",
            "S24LE 2c",
            "96000",
            Some(96000),
            "python3.13",
        )];
        let stream = StreamInfo {
            app: Some("High Tide".into()),
            ..Default::default()
        };
        assert_eq!(
            graph.assess_pcm_path(&rows, &hw, &stream).status,
            PathStatus::Clear
        );
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
}
