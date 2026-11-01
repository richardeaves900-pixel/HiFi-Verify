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
    pub processing: Vec<String>,
}

pub fn build_audit(fast: bool) -> Audit {
    let graph = telemetry::PwGraph::load();
    let hw = read_alsa_hw_params();

    let (pt_rate, pt_quant, total_err) = if fast {
        (None, None, 0u64)
    } else {
        let (_, r, q, e) = crate::pwtop::parse_pw_top();
        (r, q, e)
    };

    let dr_rate = pt_rate
        .or(graph.driver_rate)
        .or_else(|| hw.first().map(|h| h.rate));
    let dr_quant = pt_quant.or(graph.driver_quantum);

    let stream = telemetry::mpd_query()
        .or_else(telemetry::mpris_query)
        .or_else(|| graph.primary_stream());

    let mut findings = Vec::new();
    let mut fixes = Vec::new();

    let src_sr = stream.as_ref().and_then(|s| s.sr);
    match (src_sr, dr_rate) {
        (Some(src), Some(drv)) if src == drv => findings.push(Finding {
            severity: "ok",
            check: "Rate match",
            detail: format!("Source {}Hz matches graph {}Hz", src, drv),
        }),
        (Some(src), Some(drv)) => {
            findings.push(Finding {
                severity: "warn",
                check: "Rate mismatch",
                detail: format!("Source {}Hz is being converted to {}Hz", src, drv),
            });
            fixes.push(Fix {
                label: format!("Force graph rate to {}Hz", src),
                action: format!("pw-metadata -n settings 0 clock.force-rate {}", src),
            });
        }
        (None, Some(drv)) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: format!("Source rate unknown (graph at {}Hz)", drv),
        }),
        (Some(src), None) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: format!("Source at {}Hz (graph rate unknown)", src),
        }),
        (None, None) => findings.push(Finding {
            severity: "info",
            check: "Rate",
            detail: "Could not determine source or graph rate".into(),
        }),
    }

    if graph.processing.is_empty() {
        findings.push(Finding {
            severity: "ok",
            check: "Processing",
            detail: "No DSP/filter nodes detected".into(),
        });
    } else {
        findings.push(Finding {
            severity: "warn",
            check: "Processing",
            detail: format!("Active: {}", graph.processing.join(", ")),
        });
    }

    if let Some(hp) = hw.first() {
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
            if hw_bits > src_bits {
                findings.push(Finding {
                    severity: "warn",
                    check: "Bit-depth widening",
                    detail: format!(
                        "Source {}-bit but ALSA receives {}-bit",
                        src_bits, hw_bits
                    ),
                });
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
            severity: if total_err == 0 { "ok" } else { "warn" },
            check: "XRUNs",
            detail: format!("{} xrun(s)", total_err),
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

    let blocked = findings.iter().any(|f| f.severity == "warn");
    let verdict = if stream.as_ref().and_then(|s| s.codec.as_ref()).is_none() {
        "IDLE".to_string()
    } else if blocked {
        "NOT BIT-PERFECT — see warnings".to_string()
    } else {
        "POTENTIALLY BIT-PERFECT (rate match, no processing)".to_string()
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
        processing: graph.processing.clone(),
    }
}
