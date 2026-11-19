mod alsa_proc;
mod audit;
mod cava;
mod config;
mod dac;
mod history;
mod host;
mod model;
mod network;
mod pcm_verify;
mod pwtop;
mod ramp;
mod telemetry;
mod tray;
mod ui;
mod volume;
mod vu;
mod wireplumber;

use clap::Parser;
use model::{format_rate_compact, PathStatus};

#[derive(Parser)]
#[command(version, about = "HiFi Verify — PipeWire & ALSA Audio Path Monitor")]
struct Args {
    /// One-line health summary (uses live PipeWire PCM observations)
    #[arg(long)]
    health: bool,

    /// Output JSON status for KDE Plasma widget and status bars
    #[arg(long)]
    status: bool,

    /// JSON audit of the current audio path
    #[arg(long)]
    audit: bool,

    /// Skip XRUN reporting in --audit (PCM observation still uses pw-top)
    #[arg(long)]
    fast: bool,

    /// Per-PCM hardware detail (format/rate/buffer)
    #[arg(long)]
    hw: bool,

    /// View historical XRUN logs
    #[arg(long)]
    history: bool,

    /// Scan local network for UPnP / SSDP audio renderers
    #[arg(long, name = "network-scan")]
    network_scan: bool,

    /// Trusted original PCM WAV used for an actual sample-identity check
    #[arg(long, value_name = "WAV")]
    verify_reference: Option<std::path::PathBuf>,

    /// Captured PCM WAV from a known, user-selected digital monitor point
    #[arg(long, value_name = "WAV")]
    verify_capture: Option<std::path::PathBuf>,

    /// Run the loopback ramp/null test
    #[arg(long, name = "ramp-test")]
    ramp_test: bool,

    /// Print a WirePlumber 0.5 rule using the current driver rate/quantum
    #[arg(long, name = "wireplumber")]
    wireplumber: bool,

    /// Launch the GTK4 GUI (default when no CLI flag is given)
    #[arg(long)]
    gui: bool,
}

fn build_status_json(a: &audit::Audit) -> serde_json::Value {
    let status = match a.audio_path_status {
        "idle" => PathStatus::Idle,
        "clear" => PathStatus::Clear,
        "altered" => PathStatus::Altered,
        _ => PathStatus::Unknown,
    };

    let reason = match status {
        PathStatus::Idle => "Standby".to_string(),
        PathStatus::Clear => "Connected PCM path clear (samples unverified)".to_string(),
        PathStatus::Altered => a
            .findings
            .iter()
            .find(|f| f.check == "Connected PCM route")
            .map(|f| f.detail.clone())
            .unwrap_or_else(|| "Playback path alteration observed".to_string()),
        PathStatus::Unknown => a
            .findings
            .iter()
            .find(|f| f.check == "Connected PCM route")
            .map(|f| f.detail.clone())
            .unwrap_or_else(|| "Playback PCM path not fully verified".to_string()),
    };

    let rate_hz = a.dac_pcm_rate.or(a.driver_rate).or(a.stream_pcm_rate);
    let badge = if status == PathStatus::Idle {
        "IDLE".to_string()
    } else {
        let mut parts = Vec::new();
        if let Some(r) = rate_hz {
            parts.push(format_rate_compact(r));
        }
        if let Some(ref fmt) = a.dac_pcm_format {
            parts.push(fmt.clone());
        } else if let Some(ref fmt) = a.stream_pcm_format {
            parts.push(fmt.clone());
        }
        if parts.is_empty() {
            status.short_label().to_string()
        } else {
            parts.join(" ")
        }
    };

    let stream_pcm = a
        .stream_pcm_format
        .as_ref()
        .zip(a.stream_pcm_rate)
        .map(|(fmt, rate)| {
            let ch = a
                .stream_pcm_channels
                .map_or_else(|| "?".into(), |c| c.to_string());
            format!("{fmt} {} {ch}ch", format_rate_compact(rate))
        });

    let dac_pcm = a
        .dac_pcm_format
        .as_ref()
        .zip(a.dac_pcm_rate)
        .map(|(fmt, rate)| {
            let ch = a
                .dac_pcm_channels
                .map_or_else(|| "?".into(), |c| c.to_string());
            format!("{fmt} {} {ch}ch", format_rate_compact(rate))
        });

    serde_json::json!({
        "state": status.widget_state(),
        "status": status.as_str(),
        "short_label": status.short_label(),
        "badge": badge,
        "reason": reason,
        "app": a.stream_app,
        "track": a.stream_track,
        "codec": a.stream_codec,
        "stream_pcm": stream_pcm,
        "dac_pcm": dac_pcm,
        "rate_hz": rate_hz,
        "quantum": a.driver_quantum,
        "route_dsp": a.route_dsp,
        "processing": a.processing,
    })
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();

    if args.hw {
        let hw = alsa_proc::read_alsa_hw_params();
        if hw.is_empty() {
            println!("IDLE — no open playback PCM");
        } else {
            for h in &hw {
                let buf = h
                    .buffer_size
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "—".into());
                println!(
                    "HW: {} @ {}Hz, {}ch, buffer {}  ({})",
                    h.format,
                    h.rate,
                    h.channels.unwrap_or(0),
                    buf,
                    h.path
                );
            }
        }
        return std::process::ExitCode::SUCCESS;
    }

    if args.status {
        let audit = audit::build_audit(true);
        println!("{}", build_status_json(&audit));
        return std::process::ExitCode::SUCCESS;
    }

    if args.health {
        let a = audit::build_audit(true);
        let warns = a.findings.iter().filter(|f| f.severity == "warn").count();
        let rate_str = a
            .driver_rate
            .map(|r| format!("{:.1}kHz", r as f64 / 1000.0))
            .unwrap_or_else(|| "—".into());
        let quant_str = a
            .driver_quantum
            .map(|q| format!("Q{}", q))
            .unwrap_or_else(|| "—".into());
        let codec_str = a
            .stream_codec
            .clone()
            .or_else(|| a.stream_app.clone())
            .unwrap_or_else(|| "IDLE".into());
        println!(
            "{} | {} | {} | {}",
            codec_str,
            rate_str,
            quant_str,
            if warns == 0 {
                "clean".into()
            } else {
                format!("{} warning(s)", warns)
            }
        );
        return std::process::ExitCode::SUCCESS;
    }

    if args.audit {
        let a = audit::build_audit(args.fast);
        println!("{}", serde_json::to_string_pretty(&a).unwrap());
        return std::process::ExitCode::SUCCESS;
    }

    if args.history {
        println!("{}", history::get_history_summary());
        return std::process::ExitCode::SUCCESS;
    }

    if args.network_scan {
        let devices = network::scan_network();
        if devices.is_empty() {
            println!("No UPnP media renderers discovered on the local subnet.");
        } else {
            for d in devices {
                println!("{d}");
            }
        }
        return std::process::ExitCode::SUCCESS;
    }

    if args.verify_reference.is_some() || args.verify_capture.is_some() {
        match (&args.verify_reference, &args.verify_capture) {
            (Some(reference), Some(capture)) => {
                let result = pcm_verify::verify_wav(reference, capture);
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
                return if result["status"] == "pass" {
                    std::process::ExitCode::SUCCESS
                } else {
                    std::process::ExitCode::from(1)
                };
            }
            _ => {
                eprintln!("Provide BOTH --verify-reference and --verify-capture WAV files");
                return std::process::ExitCode::from(2);
            }
        }
    }

    if args.ramp_test {
        println!(
            "{}",
            serde_json::to_string_pretty(&ramp::run(48000)).unwrap()
        );
        return std::process::ExitCode::SUCCESS;
    }

    if args.wireplumber {
        let a = audit::build_audit(true);
        let rule = wireplumber::generate_rule("alsa_output.*", a.driver_rate, a.driver_quantum);
        println!("{}", rule);
        return std::process::ExitCode::SUCCESS;
    }

    let _ = args.gui;
    use ksni::blocking::TrayMethods;
    let (tx, rx) = std::sync::mpsc::channel();
    match tray::HiFiTray::new(tx)
        .disable_dbus_name(host::in_flatpak())
        .spawn()
    {
        Ok(handle) => {
            eprintln!("HiFi Verify: system tray registered");
            ui::run(Some(rx), Some(handle));
        }
        Err(err) => {
            eprintln!("HiFi Verify: tray unavailable ({err:?}); continuing without it");
            ui::run(None, None);
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_json_exposes_widget_at_a_glance_fields() {
        let a = audit::Audit {
            verdict: "PCM PATH CLEAR".into(),
            findings: vec![],
            fixes: vec![],
            hw_params: vec![],
            driver_rate: Some(96_000),
            driver_quantum: Some(512),
            stream_codec: Some("FLAC".into()),
            stream_app: Some("Strawberry".into()),
            stream_track: Some("Test Track".into()),
            stream_sr: Some(96_000),
            stream_bps: Some(24),
            stream_pcm_format: Some("S24LE".into()),
            stream_pcm_rate: Some(96_000),
            stream_pcm_channels: Some(2),
            dac_pcm_format: Some("S32_LE".into()),
            dac_pcm_rate: Some(96_000),
            dac_pcm_channels: Some(2),
            pcm_format_status: "match",
            audio_path_status: "clear",
            source_processing: None,
            source_processing_status: "unknown",
            source_metadata_status: "complete",
            processing: vec![],
            route_dsp: vec![],
        };
        let json = build_status_json(&a);
        assert_eq!(json["state"], "green");
        assert_eq!(json["short_label"], "CLEAR");
        assert_eq!(json["badge"], "96kHz S32_LE");
        assert_eq!(json["dac_pcm"], "S32_LE 96kHz 2ch");
        assert_eq!(json["stream_pcm"], "S24LE 96kHz 2ch");
        assert_eq!(json["quantum"], 512);
    }
}
