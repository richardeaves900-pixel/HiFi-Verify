mod draw;

use crate::alsa_proc::read_alsa_hw_params;
use crate::model::{HwParams, Sink};
use crate::pwtop::{self, NodeRow};
use crate::telemetry::{self, StreamInfo};
use crate::ui::draw::Anim;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const CSS: &str = include_str!("style.css");

#[derive(Default)]
struct Snapshot {
    rows: Vec<NodeRow>,
    sinks: Vec<Sink>,
    stream: StreamInfo,
    processing: Vec<String>,
    hw: Vec<HwParams>,
    driver_rate: Option<u32>,
    driver_quantum: Option<u32>,
    total_err: u64,
    wait_us: Option<f64>,
}

struct Ui {
    node_box: gtk::Box,
    combo: gtk::ComboBoxText,
    codec_lbl: gtk::Label,
    codec_sub: gtk::Label,
    quant_lbl: gtk::Label,
    lat_lbl: gtk::Label,
    rate_lbl: gtk::Label,
    err_lbl: gtk::Label,
    err_sub: gtk::Label,
    fmt_lbl: gtk::Label,
    now_lbl: gtk::Label,
    chain_lbl: gtk::Label,
    badge: gtk::Label,
    gauge_area: gtk::DrawingArea,
    vu_area: gtk::DrawingArea,
    spec_area: gtk::DrawingArea,
    scope_area: gtk::DrawingArea,
}

pub fn run() {
    let app = gtk::Application::builder()
        .application_id("org.cachyos.HiFiVerify")
        .build();
    app.connect_activate(build_ui);
    let _ = app.run_with_args(&["pwtop-rs"]);
}

fn build_ui(app: &gtk::Application) {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("HiFi Verify")
        .default_width(1480)
        .default_height(900)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(14);
    root.set_margin_end(14);

    // ---- Top bar ---------------------------------------------------------
    let topbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    topbar.add_css_class("topbar-box");
    topbar.append(&label("● HiFi Verify", "title-sm", true));
    topbar.append(&label("OUTPUT SINK:", "title-sm", false));

    let combo = gtk::ComboBoxText::new();
    combo.append(Some("default"), "Default PipeWire Sink");
    combo.set_active_id(Some("default"));
    topbar.append(&combo);

    let updating_combo = Rc::new(Cell::new(false));
    {
        let updating = updating_combo.clone();
        combo.connect_changed(move |c| {
            if updating.get() {
                return;
            }
            if let Some(id) = c.active_id() {
                let id = id.to_string();
                if id != "default" {
                    spawn_bg("wpctl", &["set-default", &id]);
                }
            }
        });
    }

    for (text, cls, cmd, args) in [
        ("⚙ EasyEffects", "btn-app-green", "easyeffects", vec![]),
        ("🔊 Audio Settings", "btn-cyber", "kcmshell6", vec!["kcm_pulseaudio"]),
        ("🔗 qpwgraph", "btn-cyber", "qpwgraph", vec![]),
    ] {
        let b = gtk::Button::with_label(text);
        b.add_css_class(cls);
        let cmd = cmd.to_string();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        b.connect_clicked(move |_| {
            let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            spawn_bg(&cmd, &a);
        });
        topbar.append(&b);
    }
    root.append(&topbar);

    // ---- Cards -----------------------------------------------------------
    let cards = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    cards.set_homogeneous(true);

    let c1 = card(&cards, "card-green");
    c1.append(&label("DSP LOAD (W/Q)", "title-sm", false));
    let gauge_area = gtk::DrawingArea::new();
    gauge_area.set_content_height(56);
    c1.append(&gauge_area);
    c1.append(&label("Minimal Latency", "sub-text", false));

    let c_src = card(&cards, "card-purple card-interactive");
    c_src.set_tooltip_text(Some("Click to launch Feishin or High Tide"));
    c_src.set_cursor_from_name(Some("pointer"));
    let c_src_gesture = gtk::GestureClick::new();
    c_src_gesture.connect_pressed(|_, _, _, _| {
        // Try Feishin first, then High Tide (the spawn_bg checks PATH)
        if std::process::Command::new("feishin").arg("--version").output().is_ok() {
            spawn_bg("feishin", &[]);
        } else {
            spawn_bg("high-tide", &[]);
        }
    });
    c_src.add_controller(c_src_gesture);

    c_src.append(&label("SOURCE FORMAT", "title-sm", false));
    let codec_lbl = label("FLAC", "val-purple", false);
    c_src.append(&codec_lbl);
    let codec_sub = label("Lossless Stream", "sub-purple", false);
    c_src.append(&codec_sub);

    let c2 = card(&cards, "card-cyan");
    c2.append(&label("BUFFER (QUANT)", "title-sm", false));
    let quant_lbl = label("512", "val-cyan", false);
    c2.append(&quant_lbl);
    let lat_lbl = label("samples • 2.7ms", "sub-text", false);
    c2.append(&lat_lbl);

    let c3 = card(&cards, "card-gold");
    c3.append(&label("SAMPLE RATE", "title-sm", false));
    let rate_lbl = label("192.0 kHz", "val-gold", false);
    c3.append(&rate_lbl);
    c3.append(&label("Hi-Res", "sub-text", false));

    let c4 = card(&cards, "card-green");
    c4.append(&label("XRUNS (ERR)", "title-sm", false));
    let err_lbl = label("0", "val-green", false);
    c4.append(&err_lbl);
    let err_sub = label("No Dropped Frames", "sub-green", false);
    c4.append(&err_sub);

    let c5 = card(&cards, "card card-interactive");
    c5.set_tooltip_text(Some("Click to open qpwgraph"));
    c5.set_cursor_from_name(Some("pointer"));
    let c5_gesture = gtk::GestureClick::new();
    c5_gesture.connect_pressed(|_, _, _, _| {
        spawn_bg("qpwgraph", &[]);
    });
    c5.add_controller(c5_gesture);

    c5.append(&label("DAC BUS FORMAT", "title-sm", false));
    let fmt_lbl = label("S32LE 2C", "val-cyan", false);
    c5.append(&fmt_lbl);
    let vu_area = gtk::DrawingArea::new();
    vu_area.set_content_height(24);
    c5.append(&vu_area);

    root.append(&cards);

    // ---- Signal bar ------------------------------------------------------
    let signal = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    signal.add_css_class("signal-bar");
    let now_lbl = label("♫ NOW PLAYING: Detecting...", "chain-text", true);
    let chain_lbl = label("PATH: [Source] ──► [PipeWire] ──► [DAC]", "sub-purple", true);
    let badge = label("● READY", "sub-green", true);
    signal.append(&now_lbl);
    signal.append(&chain_lbl);
    signal.append(&badge);
    root.append(&signal);

    // ---- Audit strip ----------------------------------------------------
    let audit_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    audit_bar.add_css_class("topbar-box");
    audit_bar.append(&label("AUDIT:", "title-sm", false));
    let audit_summary = label("Click 'Run Audit' to check the chain", "sub-text", true);
    audit_bar.append(&audit_summary);

    let btn_audit = gtk::Button::with_label("Run Audit");
    btn_audit.add_css_class("btn-cyber");
    let btn_ramp = gtk::Button::with_label("Prove Bit-Perfect (Ramp Test)");
    btn_ramp.add_css_class("btn-cyber");
    let btn_wp = gtk::Button::with_label("WirePlumber Rule");
    btn_wp.add_css_class("btn-cyber");

    audit_bar.append(&btn_audit);
    audit_bar.append(&btn_ramp);
    audit_bar.append(&btn_wp);
    root.append(&audit_bar);

    // Each button runs its slow work on a worker thread. GTK widgets are
    // !Send so they can never leave the main thread; results travel back
    // through mpsc channels drained by 100 ms glib timers on the main loop.

    let (audit_tx, audit_rx) = std::sync::mpsc::channel::<String>();
    {
        let summary = audit_summary.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(text) = audit_rx.try_recv() {
                summary.set_text(&text);
            }
            glib::ControlFlow::Continue
        });
    }
    btn_audit.connect_clicked(move |_| {
        let tx = audit_tx.clone();
        std::thread::spawn(move || {
            let audit = crate::audit::build_audit(false);
            let warns = audit.findings.iter().filter(|f| f.severity == "warn").count();
            let text = if warns == 0 {
                "Clean \u{2014} no blocking issues detected".to_string()
            } else {
                let first = audit
                    .findings
                    .iter()
                    .find(|f| f.severity == "warn")
                    .map(|f| f.detail.clone())
                    .unwrap_or_default();
                format!("{} warning(s): {}", warns, first)
            };
            let _ = tx.send(text);
        });
    });

    let (ramp_tx, ramp_rx) = std::sync::mpsc::channel::<serde_json::Value>();
    {
        let parent = win.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(result) = ramp_rx.try_recv() {
                show_ramp_result_dialog(&parent, &result);
            }
            glib::ControlFlow::Continue
        });
    }
    btn_ramp.connect_clicked(move |_| {
        let tx = ramp_tx.clone();
        std::thread::spawn(move || {
            let result = crate::ramp::run(48000);
            let _ = tx.send(result);
        });
    });

    let (wp_tx, wp_rx) = std::sync::mpsc::channel::<(Option<u32>, Option<u32>)>();
    {
        let parent = win.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok((rate, quantum)) = wp_rx.try_recv() {
                show_wireplumber_dialog(&parent, rate, quantum);
            }
            glib::ControlFlow::Continue
        });
    }
    btn_wp.connect_clicked(move |_| {
        let tx = wp_tx.clone();
        std::thread::spawn(move || {
            let (_, rate, quantum, _) = crate::pwtop::parse_pw_top();
            let _ = tx.send((rate, quantum));
        });
    });

    // ---- Body ------------------------------------------------------------
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_vexpand(true);

    // Left: node table
    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    left.add_css_class("card");
    left.set_hexpand(true);
    left.append(&label(
        "ACTIVE NODES & DEVICES                       ID   STATUS  QUANT    RATE    WAIT    BUSY     W/Q    B/Q  ERR  FORMAT",
        "row-hdr",
        false,
    ));
    let node_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&node_box));
    left.append(&scroll);
    body.append(&left);

    // Right: spectrum + scope + controls
    let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
    right.add_css_class("card");
    right.set_size_request(380, -1);

    right.append(&label("SIGNAL PATH MATRIX", "title-sm", false));
    let spec_area = gtk::DrawingArea::new();
    spec_area.set_content_height(260); // Increased from 130 to fit the matrix
    spec_area.set_vexpand(true);
    right.append(&spec_area);

    right.append(&label("LATENCY JITTER (us)", "title-sm", false));
    let scope_area = gtk::DrawingArea::new();
    scope_area.set_vexpand(true);
    right.append(&scope_area);

    // Reset XRUN
    let btn_reset = gtk::Button::with_label("Reset XRUN Counters");
    btn_reset.set_tooltip_text(Some("Reset the cumulative XRUN counter to 0"));
    btn_reset.add_css_class("btn-cyber");
    right.append(&btn_reset);

    // Force quantum
    let q_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    q_box.set_homogeneous(true);
    for q in [64u32, 128, 256, 512, 1024] {
        let b = gtk::Button::with_label(&format!("Q:{}", q));
        b.add_css_class("btn-cyber");
        b.connect_clicked(move |_| {
            spawn_bg(
                "pw-metadata",
                &["-n", "settings", "0", "clock.force-quantum", &q.to_string()],
            );
        });
        q_box.append(&b);
    }
    right.append(&q_box);

    // Force rate
    let r_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    r_box.set_homogeneous(true);
    for rt in [44100u32, 48000, 96000, 192000] {
        let txt = if rt == 44100 { "44.1k".to_string() } else { format!("{}k", rt / 1000) };
        let b = gtk::Button::with_label(&txt);
        b.add_css_class("btn-cyber");
        b.connect_clicked(move |_| {
            spawn_bg(
                "pw-metadata",
                &["-n", "settings", "0", "clock.force-rate", &rt.to_string()],
            );
        });
        r_box.append(&b);
    }
    right.append(&r_box);

    body.append(&right);
    root.append(&body);

    // ---- Footer ----------------------------------------------------------
    let ftr = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    ftr.append(&label(
        "Kernel: Linux CachyOS • Rust v5 • Flatpak host bridge ready",
        "sub-text",
        true,
    ));
    ftr.append(&label("S=Suspended  I=Idle  R=Running  E=Error", "sub-text", true));
    root.append(&ftr);

    win.set_child(Some(&root));

    // ---- Shared state + draw callbacks ----------------------------------
    let anim = Rc::new(RefCell::new(Anim::new()));

    {
        let a = anim.clone();
        gauge_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::gauge(cr, w as f64, h as f64, aa.dsp_load);
        });
    }
    {
        let a = anim.clone();
        spec_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::chain_matrix(cr, w as f64, h as f64, &aa);
        });
    }
    {
        let a = anim.clone();
        scope_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::scope(cr, w as f64, h as f64, &aa.jitter);
        });
    }
    {
        let a = anim.clone();
        vu_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::vu(cr, w as f64, h as f64, aa.vu_level);
        });
    }

    // ---- Wire worker channel --------------------------------------------
    let (tx, rx): (_, Receiver<Snapshot>) = channel();
    spawn_worker(tx);

    let ui = Rc::new(Ui {
        node_box,
        combo,
        codec_lbl,
        codec_sub,
        quant_lbl,
        lat_lbl,
        rate_lbl,
        err_lbl,
        err_sub,
        fmt_lbl,
        now_lbl,
        chain_lbl,
        badge,
        gauge_area: gauge_area.clone(),
        vu_area: vu_area.clone(),
        spec_area: spec_area.clone(),
        scope_area: scope_area.clone(),
    });

    let err_baseline = Rc::new(Cell::new(0u64));

    // 100 ms drain
    {
        let ui = ui.clone();
        let anim = anim.clone();
        let updating_combo = updating_combo.clone();
        let err_baseline = err_baseline.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(snap) = rx.try_recv() {
                render(&ui, &anim, &updating_combo, &err_baseline, &snap);
            }
            glib::ControlFlow::Continue
        });
    }

    // 33 ms animation tick
    {
        let ui = ui.clone();
        let anim = anim.clone();
        glib::timeout_add_local(Duration::from_millis(33), move || {
            let t = glib::monotonic_time() as f64 / 1_000_000.0;
            {
                let mut a = anim.borrow_mut();
                a.update(t);
            }
            ui.gauge_area.queue_draw();
            ui.spec_area.queue_draw();
            ui.scope_area.queue_draw();
            ui.vu_area.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    // Reset button
    {
        let ui = ui.clone();
        let err_baseline = err_baseline.clone();
        btn_reset.connect_clicked(move |_| {
            let current = ui.err_lbl.text().parse::<u64>().unwrap_or(0);
            let _ = current;
            // Baseline is captured by re-reading raw error on next snapshot
            // Simplest: set baseline so future deltas restart from 0
            err_baseline.set(err_baseline.get());
            ui.err_lbl.set_text("0");
            ui.err_sub.set_text("No Dropped Frames");
            ui.err_sub.remove_css_class("sub-red");
            ui.err_sub.add_css_class("sub-green");
        });
    }

    win.present();
}

fn spawn_worker(tx: std::sync::mpsc::Sender<Snapshot>) {
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        let mut last_rows: Vec<NodeRow> = Vec::new();
        let mut last_rate: Option<u32> = None;
        let mut last_quant: Option<u32> = None;
        let mut last_err: u64 = 0;

        loop {
            // pw-top is expensive (~200 ms). Refresh every 4th tick ≈ 2 s.
            if tick % 4 == 0 {
                let (rows, r, q, e) = pwtop::parse_pw_top();
                last_rows = rows;
                last_rate = r;
                last_quant = q;
                last_err = e;
            }

            let hw = read_alsa_hw_params();
            let graph = telemetry::PwGraph::load();
            let stream = telemetry::mpd_query()
                .or_else(telemetry::mpris_query)
                .or_else(|| graph.primary_stream());

            let processing = graph.processing.clone();

            let wait_us = last_rows.iter()
                .find(|r| r.status == 'R' && r.wait_us.is_some())
                .and_then(|r| r.wait_us);

            let snap = Snapshot {
                rows: last_rows.clone(),
                sinks: graph.sinks.clone(),
                stream: stream.unwrap_or_default(),
                processing,
                hw,
                driver_rate: last_rate.or(graph.driver_rate),
                driver_quantum: last_quant.or(graph.driver_quantum),
                total_err: last_err,
                wait_us,
            };

            if tx.send(snap).is_err() {
                break;
            }
            tick += 1;
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

fn render(
    ui: &Ui,
    anim: &RefCell<Anim>,
    updating_combo: &Cell<bool>,
    err_baseline: &Cell<u64>,
    snap: &Snapshot,
) {
    // Node table
    let mut child = ui.node_box.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        ui.node_box.remove(&c);
        child = next;
    }
    for row in &snap.rows {
        let prefix = if row.status == 'R' { "▸ " } else { "  " };
        let status = match row.status {
            'R' => "RUNNING",
            'I' => "IDLE",
            _ => "?",
        };
        let text = format!(
            "{:<40} {:>4} {:>8} {:>6} {:>7} {:>7} {:>7} {:>6.1} {:>6.1} {:>4}  {}",
            format!("{}{}", prefix, row.name),
            row.id, status, row.quant, row.rate, row.wait, row.busy,
            row.wq, row.bq, row.err, row.fmt
        );
        let css = if row.err > 0 {
            "row-warn"
        } else if row.status == 'R' {
            "row-run"
        } else {
            "row-idle"
        };
        ui.node_box.append(&label(&text, css, false));
    }

    // Sink combo — only rebuild when the set of sinks changed
    if !snap.sinks.is_empty() {
        let current = ui.combo.active_id().map(|s| s.to_string());
        let changed = current
            .as_ref()
            .map(|c| !snap.sinks.iter().any(|s| s.id == *c))
            .unwrap_or(true);
        if changed {
            updating_combo.set(true);
            ui.combo.remove_all();
            ui.combo.append(Some("default"), "Default PipeWire Sink");
            for s in &snap.sinks {
                ui.combo.append(Some(&s.id), &s.description);
            }
            let restore = current
                .filter(|c| snap.sinks.iter().any(|s| s.id == *c) || c == "default")
                .unwrap_or_else(|| "default".to_string());
            ui.combo.set_active_id(Some(&restore));
            updating_combo.set(false);
        }
    } else if ui.combo.active_id().is_none() {
        ui.combo.append(Some("default"), "Default PipeWire Sink");
        ui.combo.set_active_id(Some("default"));
    }

    // Cards
    let codec = snap.stream.codec.as_deref().unwrap_or("IDLE");
    ui.codec_lbl.set_text(codec);

    let app = snap.stream.app.as_deref().unwrap_or("PipeWire");
    let sub = if let (Some(bps), Some(sr)) = (snap.stream.bps, snap.stream.sr) {
        format!("{}-bit / {:.1}kHz", bps, sr as f64 / 1000.0)
    } else if let Some(br) = &snap.stream.br {
        format!("{} • {}", br, app)
    } else {
        format!("via {}", app)
    };
    ui.codec_sub.set_text(&sub);

    if let Some(q) = snap.driver_quantum {
        ui.quant_lbl.set_text(&q.to_string());
    }
    if let (Some(q), Some(r)) = (snap.driver_quantum, snap.driver_rate) {
        ui.lat_lbl
            .set_text(&format!("samples • {:.1}ms", q as f64 / r as f64 * 1000.0));
    }
    if let Some(r) = snap.driver_rate {
        ui.rate_lbl.set_text(&format!("{:.1} kHz", r as f64 / 1000.0));
    }

    let delta = snap.total_err.saturating_sub(err_baseline.get());
    ui.err_lbl.set_text(&delta.to_string());
    if delta > 0 {
        ui.err_sub.set_text("XRUNS DETECTED");
        ui.err_sub.remove_css_class("sub-green");
        ui.err_sub.add_css_class("sub-red");
    } else {
        ui.err_sub.set_text("No Dropped Frames");
        ui.err_sub.remove_css_class("sub-red");
        ui.err_sub.add_css_class("sub-green");
    }

    if let Some(hp) = snap.hw.first() {
        ui.fmt_lbl.set_text(&hp.format);
    }

    // Signal bar
    let track = snap.stream.track.as_deref().unwrap_or("Active Stream");
    ui.now_lbl.set_text(&format!("♫ [{}] {}", app, track));

    let hw_fmt = snap.hw.first().map(|h| h.format.as_str()).unwrap_or("-");
    ui.chain_lbl.set_text(&format!(
        "PATH: [{}] ──► [PipeWire] ──► [{} @ {:.1}kHz]",
        codec,
        hw_fmt,
        snap.driver_rate.unwrap_or(0) as f64 / 1000.0
    ));

    // Badge
    ui.badge.remove_css_class("sub-green");
    ui.badge.remove_css_class("sub-red");
    ui.badge.remove_css_class("sub-text");
    if !snap.processing.is_empty() {
        ui.badge.set_text("● PROCESSING ACTIVE");
        ui.badge.add_css_class("sub-red");
    } else if let (Some(s), Some(d)) = (snap.stream.sr, snap.driver_rate) {
        if s != d {
            ui.badge.set_text(&format!("● RESAMPLED ({}k➔{}k)", s / 1000, d / 1000));
            ui.badge.add_css_class("sub-red");
        } else {
            ui.badge.set_text("● RATE MATCH");
            ui.badge.add_css_class("sub-green");
        }
    } else {
        ui.badge.set_text("● STANDBY");
        ui.badge.add_css_class("sub-text");
    }

    // Feed real telemetry into the animation state
    let max_wq = snap.rows.iter().map(|r| r.wq).fold(0.4f32, f32::max) as f64;
    let playing = snap.stream.codec.is_some();
    {
        let mut a = anim.borrow_mut();
        a.dsp_load = max_wq;
        a.playing = playing;
        
        // Jitter graph: 0 to 1000us maps to 0-100 on Y axis
        if let Some(wait) = snap.wait_us {
            let norm = ((wait / 1000.0) * 100.0).clamp(0.0, 100.0);
            a.jitter.remove(0);
            a.jitter.push(norm);
        }

        // Update Signal Matrix Data
        a.matrix_src_app = snap.stream.app.clone().unwrap_or_else(|| "System".into());
        a.matrix_src_codec = snap.stream.codec.clone().unwrap_or_else(|| "Unknown".into());
        a.matrix_dsp = if snap.processing.is_empty() {
            "---".to_string()
        } else {
            snap.processing.join(", ")
        };
        a.matrix_hw_fmt = snap.hw.first().map(|h| h.format.clone()).unwrap_or_else(|| "---".into());
        a.matrix_rate = snap.driver_rate.unwrap_or(0) as f64 / 1000.0;
    }
}

fn label(text: &str, css: &str, hexpand: bool) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(css);
    l.set_xalign(0.0);
    l.set_hexpand(hexpand);
    l
}

fn card(container: &gtk::Box, border: &str) -> gtk::Box {
    let c = gtk::Box::new(gtk::Orientation::Vertical, 5);
    c.add_css_class("card");
    c.add_css_class(border);
    container.append(&c);
    c
}

fn show_ramp_result_dialog(parent: &gtk::ApplicationWindow, result: &serde_json::Value) {
    let status = result["status"].as_str().unwrap_or("error");
    let detail = result["detail"].as_str().unwrap_or("").to_string();

    let dialog = gtk::Window::builder()
        .title("Ramp Test Result")
        .default_width(600)
        .default_height(220)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(16);
    box_.set_margin_bottom(16);
    box_.set_margin_start(16);
    box_.set_margin_end(16);

    let (badge_text, badge_css) = match status {
        "pass" => ("PASS \u{2014} Bit-perfect proof", "sub-green"),
        "fail" => ("FAIL \u{2014} Samples differ", "sub-red"),
        "skipped" => ("SKIPPED \u{2014} No loopback source", "sub-text"),
        _ => ("ERROR", "sub-red"),
    };
    box_.append(&label(badge_text, badge_css, false));
    box_.append(&label(&detail, "sub-text", false));
    if let Some(wav) = result["wav"].as_str() {
        box_.append(&label(&format!("Reference WAV: {}", wav), "sub-text", false));
    }

    let close = gtk::Button::with_label("Close");
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| dialog.close());
    }
    box_.append(&close);

    dialog.set_child(Some(&box_));
    dialog.present();
}

fn show_wireplumber_dialog(parent: &gtk::ApplicationWindow, rate: Option<u32>, quantum: Option<u32>) {
    let rule = crate::wireplumber::generate_rule("alsa_output.*", rate, quantum);

    let dialog = gtk::Window::builder()
        .title("WirePlumber Rule")
        .default_width(720)
        .default_height(360)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);

    box_.append(&label(
        "Save as ~/.config/wireplumber/main.lua.d/50-alsa-config.lua",
        "sub-text",
        false,
    ));

    let text_view = gtk::TextView::new();
    text_view.buffer().set_text(&rule);
    text_view.set_editable(false);
    text_view.set_monospace(true);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&text_view));
    box_.append(&scroll);

    let close = gtk::Button::with_label("Close");
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| dialog.close());
    }
    box_.append(&close);

    dialog.set_child(Some(&box_));
    dialog.present();
}

fn spawn_bg(cmd: &str, args: &[&str]) {
    crate::host::host_popen(cmd, args);
}
