use crate::alsa_proc::read_alsa_hw_params;
use crate::model::HwParams;
use crate::telemetry;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Finding {
    pub severity: &'static str,
    pub check: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Fix {
    pub label: String,
    pub action: String,
}

#[derive(Debug, Serialize)]
pub struct Audit {
    pub verdict: String,
    pub findings: Vec<Finding>,
    pub fixes: Vec<Fix>,
    pub hw_params: Vec<HwParams>,
    pub driver_rate: Option<u32>,
    pub driver_quantum: Option<u32>,
    pub stream_codec: Option<String>,
    pub stream_app: Option<String>,
    pub stream_sr: Option<u32>,
    pub stream_bps: Option<u8>,
    /// Decoded PCM reported by the active PipeWire playback node.
    pub stream_pcm_format: Option<String>,
    pub stream_pcm_rate: Option<u32>,
    pub stream_pcm_channels: Option<u32>,
    /// ALSA hardware PCM for the output connected to the playing stream.
    pub dac_pcm_format: Option<String>,
    pub dac_pcm_rate: Option<u32>,
    pub dac_pcm_channels: Option<u32>,
    /// match, rate_mismatch, channel_mismatch, precision_reduction, or unknown.
    pub pcm_format_status: &'static str,
    /// clear, altered, unknown, or idle: never implies original sample identity.
    pub audio_path_status: &'static str,
    pub source_processing: Option<String>,
    /// active, reported_clear, unknown, or not_applicable.
    pub source_processing_status: &'static str,
    pub source_metadata_status: &'static str,
    pub processing: Vec<String>,
}

// The ALSA format width is not necessarily the precision of the source PCM.
// Representing 24-bit PCM in S32_LE can be lossless; sample identity is a
// separate question and is not established by this comparison.
fn pcm_width_finding(source_bits: u8, alsa_bits: u8, alsa_format: &str) -> Option<Finding> {
    if alsa_bits < source_bits {
        Some(Finding {
            severity: "warn",
            check: "Potential precision reduction",
            detail: format!(
                "Source {}-bit exceeds ALSA {} (nominal {}-bit). Precision loss is possible",
                source_bits, alsa_format, alsa_bits
            ),
        })
    } else if alsa_bits > source_bits {
        Some(Finding {
            severity: "info",
            check: "PCM sample width",
            detail: format!(
                "Source {}-bit is carried using ALSA {} ({}-bit); widening alone does not imply altered samples",
                source_bits, alsa_format, alsa_bits
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod pcm_width_tests {
    use super::*;

    #[test]
    fn flac_24_bit_to_s32_is_not_a_failure() {
        let result = pcm_width_finding(24, 32, "S32_LE").expect("format-width info");
        assert_eq!(result.severity, "info");
        assert_eq!(result.check, "PCM sample width");
    }

    #[test]
    fn reducing_nominal_width_is_a_warning() {
        let result = pcm_width_finding(24, 16, "S16_LE").expect("potential loss");
        assert_eq!(result.severity, "warn");
    }

    #[test]
    fn matching_width_does_not_establish_sample_identity() {
        assert!(pcm_width_finding(24, 24, "S24_LE").is_none());
    }
}

pub fn build_audit(fast: bool) -> Audit {
    let graph = telemetry::PwGraph::load();
    let hw = read_alsa_hw_params();

    // Fetch the negotiated playback PCM even in fast audit mode. The full
    // mode additionally evaluates XRUN totals; fast mode leaves those unknown.
    let (output_rows, _, _, total_err) = crate::pwtop::parse_pw_top();
    let stream = telemetry::detect_active_stream(&graph);
    let observed = graph.assess_pcm_path(&output_rows, &hw, &stream.clone().unwrap_or_default());
    let stream_pcm = observed.pcm.clone();
    let dr_rate = observed.output_rate;
    let dr_quant = observed.output_quantum;
    let pcm_rate = stream_pcm.as_ref().map(|p| p.rate);
    let pcm_format = stream_pcm.as_ref().map(|p| p.format.clone());
    let pcm_channels = stream_pcm.as_ref().and_then(|p| p.channels);
    let dac = graph.hardware_for_route(&hw);
    let dac_pcm_format = dac.map(|h| h.format.clone());
    let dac_pcm_rate = dac.map(|h| h.rate);
    let dac_pcm_channels = dac.and_then(|h| h.channels);
    let pcm_format_status = match (stream_pcm.as_ref(), dac) {
        (Some(p), Some(h)) if p.rate != h.rate => "rate_mismatch",
        (Some(p), Some(h))
            if p.channels.is_some() && h.channels.is_some() && p.channels != h.channels =>
        {
            "channel_mismatch"
        }
        (Some(p), Some(h))
            if p.format.starts_with('S')
                && p.format[1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u8>()
                    .ok()
                    .zip(h.bits)
                    .is_some_and(|(src, dst)| src > dst) =>
        {
            "precision_reduction"
        }
        (Some(p), Some(_)) if p.format.starts_with('S') => "match",
        _ => "unknown",
    };

    let mut findings = Vec::new();
    if let (Some(p), Some(h)) = (stream_pcm.as_ref(), dac) {
        findings.push(Finding {
            severity: if pcm_format_status == "match" {
                "ok"
            } else if pcm_format_status == "unknown" {
                "info"
            } else {
                "warn"
            },
            check: "Live PCM format comparison",
            detail: format!(
                "Playback {} / {} Hz / {}ch → DAC {} / {} Hz / {}ch; {}",
                p.format,
                p.rate,
                p.channels
                    .map_or_else(|| "?".to_string(), |v| v.to_string()),
                h.format,
                h.rate,
                h.channels
                    .map_or_else(|| "?".to_string(), |v| v.to_string()),
                pcm_format_status.replace('_', " ")
            ),
        });
    }

    let fixes = Vec::new();

    let src_sr = stream.as_ref().and_then(|s| s.sr);
    if let Some(route) = graph.unique_playback_route() {
        findings.push(Finding {
            severity: "ok",
            check: "PipeWire route",
            detail: format!(
                "Playback node {} directly linked to {}",
                route.stream_id, route.sink_name
            ),
        });
    }
    if let Some(pcm) = &stream_pcm {
        findings.push(Finding {
            severity: "info",
            check: "PipeWire playback PCM",
            detail: format!(
                "{} @ {}Hz, {} channel(s) — decoded stream, not original file codec",
                pcm.format,
                pcm.rate,
                pcm.channels
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
        });
    }
    match (src_sr, dr_rate) {
        (Some(src), Some(drv)) if src == drv => findings.push(Finding {
            severity: "ok",
            check: "Source/output rate observation",
            detail: format!(
                "Source {}Hz; observed output {}Hz (direct PipeWire route)",
                src, drv
            ),
        }),
        (Some(src), Some(drv)) => {
            // The graph driver rate may be from another node. A mismatch
            // here is evidence of uncertainty, not proof of resampling.
            findings.push(Finding {
                severity: "info",
                check: "Source/output rate observation",
                detail: format!(
                    "Source {}Hz; observed output {}Hz (direct PipeWire route)",
                    src, drv
                ),
            });
        }
        (None, Some(drv)) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: format!(
                "Original source rate unavailable; observed output {}Hz",
                drv
            ),
        }),
        (Some(src), None) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: format!("Source at {}Hz (output rate unavailable)", src),
        }),
        (None, None) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: "Original source and active output rates unavailable".into(),
        }),
    }

    if graph.processing.is_empty() {
        findings.push(Finding {
            severity: "ok",
            check: "Processing",
            detail: "No PipeWire DSP/filter nodes observed (upstream processing not excluded)"
                .into(),
        });
    } else {
        findings.push(Finding {
            severity: "info",
            check: "Processing",
            detail: format!(
                "DSP/filter nodes running; not proof they are on the playback route: {}",
                graph.processing.join(", ")
            ),
        });
    }

    if let Some(info) = stream.as_ref().and_then(|s| s.source_processing.as_ref()) {
        findings.push(Finding {
            severity: "warn",
            check: "Source-side processing",
            detail: info.clone(),
        });
    } else if stream.as_ref().is_some_and(|s| s.source_processing_checked) {
        findings.push(Finding {
            severity: "info",
            check: "Player gain (reported)",
            detail: "Player reports normalization off and internal playback gain unity; other processing and sample identity unverified".into(),
        });
    }

    // Without an explicit PipeWire-to-ALSA route, never arbitrarily
    // attribute the first of several active DACs to this player.
    if let (Some(src), Some(hp)) = (src_sr, graph.hardware_for_route(&hw)) {
        if hp.rate != src {
            findings.push(Finding {
                severity: "warn",
                check: "Source/ALSA rate difference",
                detail: format!(
                    "Source {}Hz, observed ALSA playback {}Hz (direct PipeWire route)",
                    src, hp.rate
                ),
            });
        }
    }

    if let Some(hp) = graph.hardware_for_route(&hw) {
        findings.push(Finding {
            severity: "info",
            check: "ALSA hw_params",
            detail: format!(
                "{} @ {}Hz, {}ch, buffer {}",
                hp.format,
                hp.rate,
                hp.channels.unwrap_or(0),
                hp.buffer_size
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "—".into())
            ),
        });
        if let (Some(src_bits), Some(hw_bits)) = (stream.as_ref().and_then(|s| s.bps), hp.bits) {
            if let Some(finding) = pcm_width_finding(src_bits, hw_bits, &hp.format) {
                findings.push(finding);
            }
        }
    } else {
        findings.push(Finding {
            severity: "info",
            check: "ALSA hw_params",
            detail: "No open playback PCM".into(),
        });
    }

    if !fast {
        findings.push(Finding {
            severity: if total_err == 0 { "ok" } else { "info" },
            check: "XRUNs",
            detail: format!("{} cumulative xrun(s); not necessarily new", total_err),
        });
    } else {
        findings.push(Finding {
            severity: "info",
            check: "XRUNs",
            detail: "Not checked (fast mode)".into(),
        });
    }

    if stream.as_ref().map_or(false, |s| s.mpd) {
        for issue in telemetry::mpd_config_check() {
            findings.push(Finding {
                severity: "warn",
                check: "MPD",
                detail: issue,
            });
        }
    }

    // This is a monitor: rate equality alone does not establish sample identity.
    // Do not penalise unknown metadata or require special player extensions.
    let source_metadata_status = match stream.as_ref() {
        None => "not_applicable",
        Some(s) if s.codec.is_some() && s.sr.is_some() && s.bps.is_some() => "complete",
        Some(s) if s.codec.is_some() || s.sr.is_some() || s.bps.is_some() => "partial",
        Some(_) => "not_exposed",
    };
    // The same route-aware decision drives audit and GUI. pw-top ERR values
    // are cumulative and are NOT treated as fresh failures on each audit.
    let audio_path_status = observed.status;
    findings.push(Finding {
        severity: match observed.status {
            "clear" => "ok",
            "altered" => "warn",
            _ => "info",
        },
        check: "Connected PCM route",
        detail: observed.explanation.clone(),
    });
    let verdict = if stream.is_none() {
        "IDLE".to_string()
    } else if audio_path_status == "clear" {
        "PCM PATH CLEAR — original sample identity not verified".to_string()
    } else if audio_path_status == "altered" {
        "AUDIO PATH ALTERATION OR RATE DIFFERENCE DETECTED".to_string()
    } else {
        "UNVERIFIED — insufficient evidence to confirm bit-perfect audio".to_string()
    };

    Audit {
        verdict,
        findings,
        fixes,
        hw_params: hw,
        driver_rate: dr_rate,
        driver_quantum: dr_quant,
        stream_codec: stream.as_ref().and_then(|s| s.codec.clone()),
        stream_app: stream.as_ref().and_then(|s| s.app.clone()),
        stream_sr: src_sr,
        stream_bps: stream.as_ref().and_then(|s| s.bps),
        stream_pcm_format: pcm_format,
        stream_pcm_rate: pcm_rate,
        stream_pcm_channels: pcm_channels,
        dac_pcm_format,
        dac_pcm_rate,
        dac_pcm_channels,
        pcm_format_status,
        audio_path_status,
        source_processing: stream.as_ref().and_then(|s|
            s.source_processing.clone().or_else(|| s.source_processing_checked.then(||
                "Player reports normalization off and internal gain unity (other processing unverified)".to_string()))),
        source_metadata_status,
        source_processing_status: match stream.as_ref() {
            None => "not_applicable",
            Some(s) if s.source_processing.is_some() => "active",
            Some(s) if s.source_processing_checked => "reported_clear",
            Some(_) => "unknown",
        },
        processing: graph.processing.clone(),
    }
}
