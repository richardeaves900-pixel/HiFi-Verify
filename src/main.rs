mod alsa_proc;
mod audit;
mod cava;
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
mod wireplumber;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "HiFi Verify")]
struct Args {
    /// One-line health summary (uses live PipeWire PCM observations)
    #[arg(long)]
    health: bool,

    /// Output JSON status for KDE widget
    #[arg(long)]
    status: bool,

    /// JSON audit of the current audio path
    #[arg(long)]
    audit: bool,

    /// Skip XRUN evaluation in --audit (PCM observation still uses pw-top)
    #[arg(long)]
    fast: bool,

    /// Per-PCM hardware detail (format/rate/buffer)
    #[arg(long)]
    hw: bool,

    /// View historical XRUN logs
    #[arg(long)]
    history: bool,

    /// Trusted original PCM WAV used for an actual sample-identity check
    #[arg(long, value_name = "WAV")]
    verify_reference: Option<std::path::PathBuf>,

    /// Captured PCM WAV from a known, user-selected digital monitor point
    #[arg(long, value_name = "WAV")]
    verify_capture: Option<std::path::PathBuf>,

    /// Run the ramp/null test
    #[arg(long, name = "ramp-test")]
    ramp_test: bool,

    /// Print a WirePlumber rule using the current driver rate/quantum
    #[arg(long, name = "wireplumber")]
    wireplumber: bool,

    /// GUI (Phase 3)
    #[arg(long)]
    gui: bool,
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
        let (state, reason) = match audit.audio_path_status {
            "idle" => ("gray", "Standby".to_string()),
            "clear" => (
                "green",
                "Connected PCM path clear (samples unverified)".to_string(),
            ),
            "altered" => (
                "red",
                audit
                    .findings
                    .iter()
                    .find(|f| f.check == "Connected PCM route")
                    .map(|f| f.detail.clone())
                    .unwrap_or_else(|| "Playback path alteration observed".to_string()),
            ),
            _ => ("amber", "Playback PCM path not fully verified".to_string()),
        };
        // Always serialize: player/process names can contain quotes or Unicode.
        println!("{}", serde_json::json!({"state": state, "reason": reason}));
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

    // --gui is retained as a compatible alias; every non-CLI launch now
    // registers the KDE StatusNotifier tray instead of only --gui launches.
    let _ = args.gui;
    use ksni::blocking::TrayMethods;
    let (tx, rx) = std::sync::mpsc::channel();
    match tray::HiFiTray::new(tx).spawn() {
        Ok(handle) => {
            eprintln!("HiFi Verify: system tray registered");
            ui::run(Some(rx), Some(handle));
        }
        Err(err) => {
            // An unavailable D-Bus tray must not stop the desktop app.
            eprintln!("HiFi Verify: tray unavailable ({err:?}); continuing without it");
            ui::run(None, None);
        }
    }
    std::process::ExitCode::SUCCESS
}
