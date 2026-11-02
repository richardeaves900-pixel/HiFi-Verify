mod alsa_proc;
mod audit;
mod cava;
mod dac;
mod history;
mod host;
mod model;
mod network;
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
    /// One-line health summary (fast, no pw-top)
    #[arg(long)]
    health: bool,

    /// Output JSON status for KDE widget
    #[arg(long)]
    status: bool,

    /// JSON audit of the current audio path
    #[arg(long)]
    audit: bool,

    /// Skip pw-top in --audit (fast mode, no XRUN data)
    #[arg(long)]
    fast: bool,

    /// Per-PCM hardware detail (format/rate/buffer)
    #[arg(long)]
    hw: bool,

    /// View historical XRUN logs
    #[arg(long)]
    history: bool,

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
        let a = audit::build_audit(true);

        let mut state = "red";
        let mut reason = "Compromised";

        if a.verdict.contains("IDLE") {
            state = "gray";
            reason = "Standby";
        } else if a.verdict.contains("POTENTIALLY BIT-PERFECT") {
            state = "green";
            reason = "Rate match (unverified)";
        } else if a.verdict.starts_with("UNVERIFIED") {
            state = "amber";
            reason = "Source sample rate unknown";
        } else {
            // If it's not bit-perfect, figure out why for the tooltip
            if !a.processing.is_empty() {
                reason = &a.processing[0]; // e.g., "easyeffects_sink"
            } else if a.verdict.contains("Rate mismatch") {
                reason = "Resampling";
            }
        }

        // Output strict JSON
        println!("{{\"state\": \"{}\", \"reason\": \"{}\"}}", state, reason);
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
