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
    path: Option<telemetry::PcmPathAssessment>,
    processing: Vec<String>,
    hw: Vec<HwParams>,
    dac_hw: Option<HwParams>,
    driver_rate: Option<u32>,
    driver_quantum: Option<u32>,
    total_err: u64,
    wait_us: Option<f64>,
    volume: Option<f64>,
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
    gauge_area: gtk::DrawingArea,
    vu_area: gtk::DrawingArea,
    spec_area: gtk::DrawingArea,
    scope_area: gtk::DrawingArea,
}

pub fn run(
    tray_rx: Option<std::sync::mpsc::Receiver<()>>,
    tray_handle: Option<ksni::blocking::Handle<crate::tray::HiFiTray>>,
) {
    let app = gtk::Application::builder()
        .application_id(if std::env::var_os("HIFI_VERIFY_DEV").is_some() {
            "org.cachyos.HiFiVerify.Dev"
        } else {
            "org.cachyos.HiFiVerify"
        })
        .build();

    let tray_rx_shared = std::sync::Arc::new(std::sync::Mutex::new(tray_rx));
    let tray_handle_shared = std::sync::Arc::new(std::sync::Mutex::new(tray_handle));

    app.connect_activate(move |app| {
        if let Some(win) = app.windows().first() {
            win.set_visible(true);
            win.present();
            return;
        }
        let rx = tray_rx_shared.lock().unwrap().take();
        let th = tray_handle_shared.lock().unwrap().take();
        build_ui(app, rx, th);
    });
    let _ = app.run_with_args(&["hifi-verify"]);
}

fn build_ui(
    app: &gtk::Application,
    tray_rx: Option<std::sync::mpsc::Receiver<()>>,
    tray_handle: Option<ksni::blocking::Handle<crate::tray::HiFiTray>>,
) {
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
        .default_width(720)
        .default_height(430)
        .build();

    // A real draggable title bar and a scrollable interior: the window
    // remains usable on small laptops, scaled displays and tiled desktops.
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("HiFi Verify"))));
    win.set_titlebar(Some(&header));
    win.set_resizable(true);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 5);
    root.set_margin_top(7);
    root.set_margin_bottom(7);
    root.set_margin_start(8);
    root.set_margin_end(8);

    // ---- Top bar ---------------------------------------------------------
    // Compact now-playing bar; infrequently used controls live in an expander.
    let topbar = gtk::Box::new(gtk::Orientation::Vertical, 3);
    topbar.add_css_class("topbar-box");
    let top_first = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let tools_expander = gtk::Expander::new(Some("Devices & tools"));
    tools_expander.set_expanded(false);
    let tools_content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let tools_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let top_links = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    tools_row.append(&label("OUTPUT SINK:", "title-sm", false));

    let combo = gtk::ComboBoxText::new();
    combo.append(Some("default"), "Default PipeWire Sink");
    combo.set_active_id(Some("default"));
    tools_row.append(&combo);

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

    let btn_net = gtk::Button::with_label("🌐 Scan Network");
    btn_net.add_css_class("btn-cyber");
    let (net_tx, net_rx) = std::sync::mpsc::channel::<Vec<String>>();
    {
        let net_tx = net_tx.clone();
        btn_net.connect_clicked(move |_| {
            let tx = net_tx.clone();
            std::thread::spawn(move || {
                let devices = crate::network::scan_network();
                let _ = tx.send(devices);
            });
        });
    }
    tools_row.append(&btn_net);

    // Hardware Volume Control
    let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let vol_icon = gtk::Label::new(Some("🔊"));
    let vol_slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    vol_slider.set_draw_value(false);
    vol_slider.set_size_request(84, -1);
    vol_slider.set_value(crate::volume::get_volume());
    vol_slider.set_tooltip_text(Some("PipeWire sink volume (may use software gain)"));

    // GTK must not wait for wpctl on each slider movement. A dedicated worker
    // consumes pending values, keeping only the most recent during a drag.
    let updating_volume = Rc::new(Cell::new(false));
    let (volume_tx, volume_rx) = std::sync::mpsc::channel::<f64>();
    std::thread::spawn(move || {
        while let Ok(mut value) = volume_rx.recv() {
            while let Ok(newer) = volume_rx.try_recv() {
                value = newer;
            }
            crate::volume::set_volume(value);
        }
    });
    {
        let updating_volume = updating_volume.clone();
        vol_slider.connect_value_changed(move |s| {
            if !updating_volume.get() {
                let _ = volume_tx.send(s.value());
            }
        });
    }

    vol_box.append(&vol_icon);
    vol_box.append(&vol_slider);
    top_first.append(&vol_box);

    // Media Controls
    let media_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    media_box.add_css_class("btn-cyber");

    let btn_prev = gtk::Button::with_label("⏮");
    btn_prev.set_tooltip_text(Some("Previous Track"));
    btn_prev.connect_clicked(|_| crate::telemetry::send_mpris_command("Previous"));
    media_box.append(&btn_prev);

    let btn_play = gtk::Button::with_label("⏯");
    btn_play.set_tooltip_text(Some("Play / Pause"));
    btn_play.connect_clicked(|_| crate::telemetry::send_mpris_command("PlayPause"));
    media_box.append(&btn_play);

    let btn_next = gtk::Button::with_label("⏭");
    btn_next.set_tooltip_text(Some("Next Track"));
    btn_next.connect_clicked(|_| crate::telemetry::send_mpris_command("Next"));
    media_box.append(&btn_next);

    top_first.append(&media_box);

    // Now Playing Text in the top bar
    let now_lbl = label("♫ NOW PLAYING: Detecting...", "chain-text", true);
    now_lbl.set_margin_start(4);
    now_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    now_lbl.set_hexpand(true);
    now_lbl.set_max_width_chars(42);
    top_first.append(&now_lbl);

    let btn_dac = gtk::Button::with_label("🔍 DAC Caps");
    btn_dac.add_css_class("btn-cyber");
    let (dac_tx, dac_rx) = std::sync::mpsc::channel::<(Vec<String>, Vec<String>, Vec<String>)>();
    {
        let button = btn_dac.clone();
        btn_dac.connect_clicked(move |_| {
            button.set_sensitive(false);
            let tx = dac_tx.clone();
            std::thread::spawn(move || {
                let result = crate::dac::get_dac_capabilities();
                let _ = tx.send(result);
            });
        });
    }
    tools_row.append(&btn_dac);

    for (text, cls, cmd, args) in [
        ("⚙ EasyEffects", "btn-app-green", "easyeffects", vec![]),
        (
            "🔊 Audio Settings",
            "btn-cyber",
            "kcmshell6",
            vec!["kcm_pulseaudio"],
        ),
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
        top_links.append(&b);
    }
    topbar.append(&top_first);
    root.append(&topbar);
    // Horizontal scroll prevents long DAC names and tool buttons from inflating
    // the window's minimum width. All functionality stays one click away.
    let device_scroll = gtk::ScrolledWindow::new();
    device_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    device_scroll.set_child(Some(&tools_row));
    tools_content.append(&device_scroll);
    let links_scroll = gtk::ScrolledWindow::new();
    links_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    links_scroll.set_child(Some(&top_links));
    tools_content.append(&links_scroll);
    let extra_tools = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let extra_scroll = gtk::ScrolledWindow::new();
    extra_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    extra_scroll.set_child(Some(&extra_tools));
    tools_content.append(&extra_scroll);
    tools_expander.set_child(Some(&tools_content));
    root.append(&tools_expander);

    // ---- Cards -----------------------------------------------------------
    let cards = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let cards_row_one = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    cards_row_one.set_homogeneous(true);
    let cards_row_two = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    cards_row_two.set_homogeneous(true);

    let c1 = card(&cards_row_one, "card-green");
    c1.append(&label("DSP LOAD (W/Q)", "title-sm", false));
    let gauge_area = gtk::DrawingArea::new();
    gauge_area.set_content_height(25);
    c1.append(&gauge_area);
    c1.append(&label("Minimal Latency", "sub-text", false));

    let c_src = card(&cards_row_one, "card-purple card-interactive");
    c_src.set_tooltip_text(Some("Click to launch Feishin or High Tide"));
    c_src.set_cursor_from_name(Some("pointer"));
    let c_src_gesture = gtk::GestureClick::new();
    c_src_gesture.connect_pressed(|_, _, _, _| {
        // Try Feishin first, then High Tide (the spawn_bg checks PATH)
        if std::process::Command::new("feishin")
            .arg("--version")
            .output()
            .is_ok()
        {
            spawn_bg("feishin", &[]);
        } else {
            spawn_bg("high-tide", &[]);
        }
    });
    c_src.add_controller(c_src_gesture);

    c_src.append(&label("PLAYER / SOURCE FORMAT", "title-sm", false));
    let codec_lbl = label("FLAC", "val-purple", false);
    codec_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    codec_lbl.set_max_width_chars(24);
    c_src.append(&codec_lbl);
    let codec_sub = label("Source format unverified", "sub-purple", false);
    codec_sub.set_wrap(true);
    codec_sub.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    codec_sub.set_max_width_chars(26);
    c_src.append(&codec_sub);

    let c2 = card(&cards_row_one, "card-cyan");
    c2.append(&label("BUFFER (QUANT)", "title-sm", false));
    let quant_lbl = label("512", "val-cyan", false);
    c2.append(&quant_lbl);
    let lat_lbl = label("samples • 2.7ms", "sub-text", false);
    c2.append(&lat_lbl);

    let c3 = card(&cards_row_two, "card-gold");
    c3.append(&label("ACTIVE OUTPUT RATE", "title-sm", false));
    let rate_lbl = label("192.0 kHz", "val-gold", false);
    c3.append(&rate_lbl);
    c3.append(&label("Hi-Res", "sub-text", false));

    let c4 = card(&cards_row_two, "card-green");
    c4.append(&label("XRUNS (ERR)", "title-sm", false));
    let err_lbl = label("0", "val-green", false);
    c4.append(&err_lbl);
    let err_sub = label("No Dropped Frames", "sub-green", false);
    c4.append(&err_sub);

    let c5 = card(&cards_row_two, "card card-interactive");
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
    vu_area.set_content_height(13);
    c5.append(&vu_area);

    cards.append(&cards_row_one);
    cards.append(&cards_row_two);
    root.append(&cards);

    // ---- Audit strip ----------------------------------------------------
    let audit_bar = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    audit_bar.add_css_class("topbar-box");
    audit_bar.append(&label("AUDIT:", "title-sm", false));
    let audit_summary = label("Click Run Audit to check the chain", "sub-text", true);
    audit_summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    audit_summary.set_max_width_chars(40);
    audit_bar.append(&audit_summary);

    let btn_audit = gtk::Button::with_label("Run Audit");
    btn_audit.add_css_class("btn-cyber");
    let btn_ramp = gtk::Button::with_label("Prove Bit-Perfect (Ramp Test)");
    btn_ramp.add_css_class("btn-cyber");

    let btn_hist = gtk::Button::with_label("📋 XRUN History");
    btn_hist.add_css_class("btn-cyber");
    {
        let parent = win.clone();
        btn_hist.connect_clicked(move |_| {
            let summary = crate::history::get_history_summary();
            show_history_dialog(&parent, &summary);
        });
    }
    let btn_wp = gtk::Button::with_label("WirePlumber Rule");
    btn_wp.add_css_class("btn-cyber");

    audit_bar.append(&btn_audit);
    extra_tools.append(&btn_ramp);
    extra_tools.append(&btn_hist);
    extra_tools.append(&btn_wp);
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
                summary.set_tooltip_text(Some(&text));
            }
            glib::ControlFlow::Continue
        });
    }
    btn_audit.connect_clicked(move |_| {
        let tx = audit_tx.clone();
        std::thread::spawn(move || {
            let audit = crate::audit::build_audit(false);
            let warns = audit
                .findings
                .iter()
                .filter(|f| f.severity == "warn")
                .count();
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
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    body.set_vexpand(true);

    // Left: node table
    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    left.add_css_class("card");
    left.set_hexpand(true);
    left.append(&label(
        "ACTIVE NODES & DEVICES — scroll horizontally for full details",
        "row-hdr",
        false,
    ));
    let node_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    scroll.set_min_content_width(260);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&node_box));
    left.append(&scroll);
    body.append(&left);

    // Right: spectrum + scope + controls
    let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
    right.add_css_class("card");
    right.set_size_request(225, -1);

    let toggle_btn = gtk::ToggleButton::with_label("SIGNAL PATH MATRIX");
    toggle_btn.add_css_class("btn-cyber");
    right.append(&toggle_btn);

    let spec_data = std::sync::Arc::new(std::sync::Mutex::new(vec![0.0; 32]));
    crate::cava::spawn_cava_worker(spec_data.clone());

    let spec_area = gtk::DrawingArea::new();
    spec_area.set_content_height(135);
    spec_area.set_vexpand(true);
    right.append(&spec_area);

    right.append(&label("LATENCY JITTER (us)", "title-sm", false));
    let scope_area = gtk::DrawingArea::new();
    scope_area.set_content_height(52);
    scope_area.set_vexpand(true);
    right.append(&scope_area);

    let btn_reset = gtk::Button::with_label("Reset XRUN Counters");
    btn_reset.set_tooltip_text(Some("Reset the cumulative XRUN counter to 0"));
    btn_reset.add_css_class("btn-cyber");
    right.append(&btn_reset);

    // Read-only monitor: do not change sample rates or buffer quanta.
    right.append(&label("Playback clocks: read-only", "sub-text", false));

    body.append(&right);
    // Large graphs and the full node table are optional, not permanent
    // space consumers in the compact dashboard.
    let details_expander = gtk::Expander::new(Some("More details: nodes & signal graphs"));
    details_expander.set_expanded(false);
    let detail_scroll = gtk::ScrolledWindow::new();
    detail_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    detail_scroll.set_min_content_height(235);
    detail_scroll.set_max_content_height(340);
    detail_scroll.set_child(Some(&body));
    details_expander.set_child(Some(&detail_scroll));
    root.append(&details_expander);

    // Static footer removed to keep the default dashboard compact.
    let viewport = gtk::ScrolledWindow::new();
    viewport.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    viewport.set_child(Some(&root));
    win.set_child(Some(&viewport));

    // ---- Shared state + draw callbacks ----------------------------------
    let anim = Rc::new(RefCell::new(Anim::new()));

    // Connect spec_area drawing (Matrix vs Spectrum toggle)
    {
        let a = anim.clone();
        let toggle_btn_draw = toggle_btn.clone();
        let spec_data_draw = spec_data.clone();
        spec_area.set_draw_func(move |_, cr, w, h| {
            if toggle_btn_draw.is_active() {
                if let Ok(bands) = spec_data_draw.try_lock() {
                    draw::spectrum(cr, w as f64, h as f64, &bands);
                }
            } else {
                let aa = a.borrow();
                draw::chain_matrix(cr, w as f64, h as f64, &aa);
            }
        });
    }

    // Connect toggle button
    {
        let spec_area_clone = spec_area.clone();
        toggle_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                btn.set_label("AUDIO SPECTRUM");
            } else {
                btn.set_label("SIGNAL PATH MATRIX");
            }
            spec_area_clone.queue_draw();
        });
    }

    // Connect scope_area drawing
    {
        let a = anim.clone();
        scope_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::scope(cr, w as f64, h as f64, &aa.jitter);
        });
    }

    // Connect gauge_area drawing
    {
        let a = anim.clone();
        gauge_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::gauge(cr, w as f64, h as f64, aa.dsp_load);
        });
    }

    // Connect vu_area drawing
    {
        let a = anim.clone();
        vu_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::vu(cr, w as f64, h as f64, aa.vu_level);
        });
    }

    {
        let a = anim.clone();
        gauge_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::gauge(cr, w as f64, h as f64, aa.dsp_load);
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
        gauge_area: gauge_area.clone(),
        vu_area: vu_area.clone(),
        spec_area: spec_area.clone(),
        scope_area: scope_area.clone(),
    });

    let err_baseline = Rc::new(Cell::new(0u64));
    let last_seen_err = Rc::new(Cell::new(0u64));
    let baseline_initialized = Rc::new(Cell::new(false));

    // 100 ms drain
    {
        let ui = ui.clone();
        let anim = anim.clone();
        let updating_combo = updating_combo.clone();
        let err_baseline = err_baseline.clone();
        let last_seen_err = last_seen_err.clone();
        let baseline_initialized = baseline_initialized.clone();
        let tray_handle_clone = tray_handle.clone();
        let updating_volume = updating_volume.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(snap) = rx.try_recv() {
                if !baseline_initialized.get() {
                    err_baseline.set(snap.total_err);
                    baseline_initialized.set(true);
                }
                last_seen_err.set(snap.total_err);
                render(&ui, &anim, &updating_combo, &err_baseline, &snap);

                // wpctl has already run in the telemetry worker, never here.
                if !vol_slider.is_focus() {
                    if let Some(current_vol) = snap.volume {
                        if (vol_slider.value() - current_vol).abs() > 0.01 {
                            updating_volume.set(true);
                            vol_slider.set_value(current_vol.clamp(0.0, 1.0));
                            updating_volume.set(false);
                        }
                    }
                }

                if let Some(ref th) = tray_handle_clone {
                    let is_playing = snap.stream.sr.is_some()
                        || snap.stream.codec.is_some()
                        || snap.stream.app.is_some();
                    let mut detected_processing = snap.processing.clone();
                    if let Some(ref source) = snap.stream.source_processing {
                        detected_processing.push(source.clone());
                    }
                    let has_dsp = !detected_processing.is_empty();
                    let observed_pcm = snap.path.as_ref().and_then(|p| p.pcm.as_ref());
                    // A cumulative pw-top error counter is not a new XRUN.
                    // The exact same graph/ALSA assessment drives CLI, GUI and tray.
                    let (health, status_label) = match snap.path.as_ref().map(|p| p.status) {
                        Some("idle") => (crate::tray::TrayHealth::Idle, "STANDBY".into()),
                        Some("clear") => (
                            crate::tray::TrayHealth::RateMatch,
                            "PCM PATH CLEAR (SAMPLES UNVERIFIED)".into(),
                        ),
                        Some("altered") => (
                            crate::tray::TrayHealth::Warning,
                            snap.path.as_ref().unwrap().explanation.clone(),
                        ),
                        _ => (crate::tray::TrayHealth::Unknown, "UNVERIFIED".into()),
                    };

                    let app_name = snap.stream.app.clone().unwrap_or_else(|| "Unknown".into());
                    let codec = snap
                        .stream
                        .codec
                        .as_deref()
                        .unwrap_or("Source codec not exposed");
                    let stream_info = match (snap.stream.sr, snap.stream.bps) {
                        (Some(rate), Some(bits)) => {
                            format!("{} · {}-bit / {:.1} kHz", codec, bits, rate as f64 / 1000.0)
                        }
                        (Some(rate), None) => {
                            format!("{} · {:.1} kHz", codec, rate as f64 / 1000.0)
                        }
                        _ => observed_pcm
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "{} | decoded PCM {} @ {:.1} kHz",
                                    codec,
                                    p.format,
                                    p.rate as f64 / 1000.0
                                )
                            })
                            .unwrap_or_else(|| codec.to_string()),
                    };
                    let pipewire_info = match (snap.driver_rate, snap.driver_quantum) {
                        (Some(rate), Some(q)) => {
                            format!("{:.1} kHz · Q{}", rate as f64 / 1000.0, q)
                        }
                        (Some(rate), None) => {
                            format!("{:.1} kHz · quantum unknown", rate as f64 / 1000.0)
                        }
                        (None, Some(q)) => format!("rate unknown · Q{}", q),
                        _ => "Unknown".into(),
                    };
                    let hw_info = snap
                        .dac_hw
                        .as_ref()
                        .map(|hw| format!("{} @ {:.1} kHz", hw.format, hw.rate as f64 / 1000.0))
                        .unwrap_or_else(|| "No open playback PCM".into());
                    let dsp_info = if has_dsp {
                        detected_processing.join(", ")
                    } else if snap.stream.source_processing_checked {
                        "High Tide gain reported clean; other processing unverified".into()
                    } else {
                        "No PipeWire DSP detected; source gain unknown".into()
                    };
                    let xrun_info = snap.total_err.to_string();

                    let _ = th.update(move |t| {
                        t.status_label = status_label;
                        t.health = health;
                        t.is_playing = is_playing;
                        t.app_name = app_name;
                        t.stream_info = stream_info;
                        t.hw_info = hw_info;
                        t.dsp_info = dsp_info;
                        t.pipewire_info = pipewire_info;
                        t.xrun_info = xrun_info;
                    });
                }
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

    // Reset only the HiFi Verify display baseline, never kernel/driver counters.
    {
        let ui = ui.clone();
        let err_baseline = err_baseline.clone();
        let last_seen_err = last_seen_err.clone();
        btn_reset.connect_clicked(move |_| {
            err_baseline.set(last_seen_err.get());
            ui.err_lbl.set_text("0");
            ui.err_sub.set_text("No new dropped frames");
            ui.err_sub.remove_css_class("sub-red");
            ui.err_sub.add_css_class("sub-green");
        });
    }

    win.set_hide_on_close(true);
    win.connect_close_request(|win| {
        win.set_visible(false);
        glib::Propagation::Stop
    });
    win.present();

    // Tray click / Show-Hide menu actually toggles the window.
    if let Some(rx) = tray_rx {
        let win_clone = win.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            while rx.try_recv().is_ok() {
                if win_clone.is_visible() {
                    win_clone.set_visible(false);
                } else {
                    win_clone.present();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Handle Network Scan Results
    {
        let win_clone = win.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            while let Ok(devices) = net_rx.try_recv() {
                show_network_dialog(&win_clone, devices);
            }
            glib::ControlFlow::Continue
        });
    }

    // Only create GTK dialogs on the main thread, after DAC work finishes.
    {
        let parent = win.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok((rates, formats, dsd)) = dac_rx.try_recv() {
                btn_dac.set_sensitive(true);
                show_dac_dialog(&parent, rates, formats, dsd);
            }
            glib::ControlFlow::Continue
        });
    }
}

fn spawn_worker(tx: std::sync::mpsc::Sender<Snapshot>) {
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        let mut last_rows: Vec<NodeRow> = Vec::new();
        let mut last_err: u64 = 0;
        let mut xrun_log_started = false;
        let mut graph = telemetry::PwGraph::default();
        let mut stream = StreamInfo::default();
        let mut volume = None;

        loop {
            // pw-top is expensive (~200 ms). Refresh every 4th tick ≈ 2 s.
            if tick % 4 == 0 {
                let (rows, _, _, e) = pwtop::parse_pw_top();

                // Log new XRUNs to SQLite
                if xrun_log_started && e > last_err {
                    crate::history::log_xruns(e - last_err);
                }
                xrun_log_started = true;

                last_rows = rows;
                last_err = e;
            }

            // The hardware format remains fresh on each snapshot. Expensive
            // process-based graph/metadata reads are cached between polls.
            let hw = read_alsa_hw_params();
            if tick % 4 == 0 {
                graph = telemetry::PwGraph::load();
            }
            if tick % 2 == 0 {
                stream = telemetry::detect_active_stream(&graph).unwrap_or_default();
                volume = crate::volume::try_get_volume();
            }

            let processing = graph.processing.clone();

            let wait_us = last_rows
                .iter()
                .find(|r| r.status == 'R' && r.wait_us.is_some())
                .and_then(|r| r.wait_us);

            // Keep output observations separate from arbitrary pw-top nodes.
            // Evaluate before `hw` is moved into the snapshot.
            let (node_rate, node_quantum) = telemetry::observed_output_clock(&last_rows);
            let path = graph.assess_pcm_path(&last_rows, &hw, &stream);
            let active_rate = path.output_rate.or(node_rate);
            let active_quantum = path.output_quantum.or(node_quantum);
            let snap = Snapshot {
                rows: last_rows.clone(),
                sinks: graph.sinks.clone(),
                stream: stream.clone(),
                path: Some(path),
                processing,
                dac_hw: graph.hardware_for_route(&hw).cloned(),
                hw,
                driver_rate: active_rate,
                driver_quantum: active_quantum,
                total_err: last_err,
                wait_us,
                volume,
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
    // Reuse existing GTK labels instead of removing/recreating every row.
    let mut child = ui.node_box.first_child();
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
            row.id,
            status,
            row.quant,
            row.rate,
            row.wait,
            row.busy,
            row.wq,
            row.bq,
            row.err,
            row.fmt
        );
        let css = if row.err > 0 {
            "row-warn"
        } else if row.status == 'R' {
            "row-run"
        } else {
            "row-idle"
        };
        if let Some(widget) = child.take() {
            child = widget.next_sibling();
            match widget.downcast::<gtk::Label>() {
                Ok(existing) => {
                    if existing.text().as_str() != text {
                        existing.set_text(&text);
                    }
                    if !existing.has_css_class(css) {
                        for class in ["row-warn", "row-run", "row-idle"] {
                            existing.remove_css_class(class);
                        }
                        existing.add_css_class(css);
                    }
                }
                Err(other) => {
                    ui.node_box.remove(&other);
                    ui.node_box.append(&label(&text, css, false));
                }
            }
        } else {
            ui.node_box.append(&label(&text, css, false));
        }
    }
    // Remove only leftover rows when the node count shrinks.
    while let Some(widget) = child {
        child = widget.next_sibling();
        ui.node_box.remove(&widget);
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

    // Show the actual audio-producing app, even if source codec is unknown.
    // Codec and bit depth remain visible separately when reliably available.
    let app_name = snap.stream.app.as_deref().filter(|s| !s.is_empty());
    let codec = snap.stream.codec.as_deref();
    ui.codec_lbl.set_text(&match (app_name, codec) {
        (Some(app), Some(fmt)) => format!("{} · {}", app, fmt),
        (Some(app), None) => app.to_string(),
        (None, Some(fmt)) => fmt.to_string(),
        (None, None) => "IDLE".to_string(),
    });

    let app = app_name.unwrap_or("Unknown application");
    let codec_name = codec.unwrap_or("Source codec not exposed");
    // Display the source metadata and BOTH live transport formats.
    // Do not let source codec availability hide the negotiated PCM details.
    let source = match (snap.stream.bps, snap.stream.sr) {
        (Some(bits), Some(rate)) => {
            format!("{} {}-bit/{:.1}kHz", codec_name, bits, rate as f64 / 1000.0)
        }
        (None, Some(rate)) => format!("{} {:.1}kHz", codec_name, rate as f64 / 1000.0),
        _ => codec_name.to_string(),
    };
    let stream_pcm = snap
        .path
        .as_ref()
        .and_then(|p| p.pcm.as_ref())
        .map(|p| {
            format!(
                "Stream {} {:.1}kHz {}ch",
                p.format,
                p.rate as f64 / 1000.0,
                p.channels
                    .map_or_else(|| "?".to_string(), |c| c.to_string())
            )
        })
        .unwrap_or_else(|| "Stream PCM unknown".to_string());
    let dac_pcm = snap
        .dac_hw
        .as_ref()
        .map(|h| {
            format!(
                "DAC {} {:.1}kHz {}ch",
                h.format,
                h.rate as f64 / 1000.0,
                h.channels
                    .map_or_else(|| "?".to_string(), |c| c.to_string())
            )
        })
        .unwrap_or_else(|| "DAC PCM unknown".to_string());
    let sub = if app_name.is_some() {
        format!("{} | {} → {}", source, stream_pcm, dac_pcm)
    } else {
        "No active playback".to_string()
    };
    let brief_source = if codec.is_some() {
        source
    } else {
        "Source unknown".to_string()
    };
    let brief_stream = snap
        .path
        .as_ref()
        .and_then(|p| p.pcm.as_ref())
        .map(|p| format!("{} {:.1}k", p.format, p.rate as f64 / 1000.0))
        .unwrap_or_else(|| "PCM ?".into());
    let brief_dac = snap
        .dac_hw
        .as_ref()
        .map(|h| format!("{} {:.1}k", h.format, h.rate as f64 / 1000.0))
        .unwrap_or_else(|| "DAC ?".into());
    ui.codec_sub.set_text(&format!(
        "{}\n{} → {}",
        brief_source, brief_stream, brief_dac
    ));
    ui.codec_sub.set_tooltip_text(Some(&sub));

    if let Some(q) = snap.driver_quantum {
        ui.quant_lbl.set_text(&q.to_string());
    }
    if let (Some(q), Some(r)) = (snap.driver_quantum, snap.driver_rate) {
        ui.lat_lbl
            .set_text(&format!("samples • {:.1}ms", q as f64 / r as f64 * 1000.0));
    }
    if let Some(r) = snap.driver_rate {
        ui.rate_lbl
            .set_text(&format!("{:.1} kHz", r as f64 / 1000.0));
    } else {
        ui.rate_lbl.set_text("—");
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

    ui.fmt_lbl.set_text(
        snap.dac_hw
            .as_ref()
            .map(|hw| hw.format.as_str())
            .unwrap_or("Unknown"),
    );

    // Signal bar
    let track = snap.stream.track.as_deref().unwrap_or("Active Stream");
    ui.now_lbl.set_text(&format!("♫ [{}] {}", app, track));

    // Check for DSP notifications
    {
        let mut a = anim.borrow_mut();
        if snap.stream.source_processing.is_some() {
            if !a.is_processing {
                a.is_processing = true;
                let _ = notify_rust::Notification::new()
                    .summary("HiFi Verify: Compromised")
                    .body(&format!("DSP active: {}", snap.processing[0]))
                    .icon("dialog-warning")
                    .show();
            }
        } else {
            a.is_processing = false;
        }
    }

    // Feed real telemetry into the animation state
    let max_wq = snap.rows.iter().map(|r| r.wq).fold(0.4f32, f32::max) as f64;
    let playing = snap.stream.codec.is_some() || snap.stream.app.is_some();
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
        a.matrix_src_codec = snap
            .stream
            .codec
            .clone()
            .unwrap_or_else(|| "Unknown".into());
        let mut processing = snap.processing.clone();
        if let Some(ref source) = snap.stream.source_processing {
            processing.push(source.clone());
        }
        a.matrix_dsp = if !processing.is_empty() {
            processing.join(", ")
        } else if snap.stream.source_processing_checked {
            "High Tide gain off / unity (reported)".to_string()
        } else {
            "Source gain unknown; PipeWire DSP none".to_string()
        };
        a.matrix_hw_fmt = snap
            .dac_hw
            .as_ref()
            .map(|h| h.format.clone())
            .unwrap_or_else(|| "---".into());
        // The DAC bus must show the hardware PCM rate, not the PipeWire graph rate.
        a.matrix_rate = snap.driver_rate.unwrap_or(0) as f64 / 1000.0;
        a.alsa_bitperfect = snap.path.as_ref().is_some_and(|p| p.status == "clear");
        a.alsa_reason = snap
            .path
            .as_ref()
            .map(|p| p.explanation.clone())
            .unwrap_or_else(|| "No verified playback path".into());
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
    let c = gtk::Box::new(gtk::Orientation::Vertical, 2);
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
        box_.append(&label(
            &format!("Reference WAV: {}", wav),
            "sub-text",
            false,
        ));
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

fn show_wireplumber_dialog(
    parent: &gtk::ApplicationWindow,
    rate: Option<u32>,
    quantum: Option<u32>,
) {
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

fn show_network_dialog(parent: &gtk::ApplicationWindow, devices: Vec<String>) {
    let dialog = gtk::Window::builder()
        .title("Network Streamers (UPnP)")
        .default_width(450)
        .default_height(300)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(16);
    box_.set_margin_bottom(16);
    box_.set_margin_start(16);
    box_.set_margin_end(16);

    let title = gtk::Label::new(Some("Discovered Media Renderers:"));
    title.set_xalign(0.0);
    title.add_css_class("title-sm");
    box_.append(&title);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);

    let list_box = gtk::ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::None);

    if devices.is_empty() {
        let lbl = gtk::Label::new(Some(
            "No devices found. Ensure your streamer is on the same subnet.",
        ));
        lbl.set_wrap(true);
        lbl.add_css_class("sub-text");
        list_box.append(&lbl);
    } else {
        for dev in devices {
            let row = gtk::Label::new(Some(&dev));
            row.set_xalign(0.0);
            row.set_margin_top(8);
            row.set_margin_bottom(8);
            row.add_css_class("val-purple");
            list_box.append(&row);
        }
    }

    scroll.set_child(Some(&list_box));
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

fn show_history_dialog(parent: &gtk::ApplicationWindow, summary: &str) {
    let dialog = gtk::Window::builder()
        .title("XRUN History (Last 50)")
        .default_width(500)
        .default_height(400)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);

    let text_view = gtk::TextView::new();
    text_view.buffer().set_text(summary);
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

fn show_dac_dialog(
    parent: &gtk::ApplicationWindow,
    rates: Vec<String>,
    formats: Vec<String>,
    dsd: Vec<String>,
) {
    let dialog = gtk::Window::builder()
        .title("DAC Hardware Capabilities")
        .default_width(500)
        .default_height(350)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(20);
    box_.set_margin_bottom(20);
    box_.set_margin_start(20);
    box_.set_margin_end(20);

    let lbl_rate = gtk::Label::new(Some("Supported Sample Rates:"));
    lbl_rate.set_xalign(0.0);
    lbl_rate.add_css_class("title-sm");
    box_.append(&lbl_rate);

    let rate_str = if rates.is_empty() {
        "Not detected".to_string()
    } else {
        rates.join(", ")
    };
    let rate_val = gtk::Label::new(Some(&rate_str));
    rate_val.set_xalign(0.0);
    rate_val.set_wrap(true);
    rate_val.add_css_class("val-cyan");
    box_.append(&rate_val);

    let lbl_fmt = gtk::Label::new(Some("Supported PCM Formats:"));
    lbl_fmt.set_xalign(0.0);
    lbl_fmt.add_css_class("title-sm");
    box_.append(&lbl_fmt);

    let fmt_str = if formats.is_empty() {
        "Not detected".to_string()
    } else {
        formats.join(", ")
    };
    let fmt_val = gtk::Label::new(Some(&fmt_str));
    fmt_val.set_xalign(0.0);
    fmt_val.set_wrap(true);
    fmt_val.add_css_class("val-purple");
    box_.append(&fmt_val);

    // DSD Section
    let lbl_dsd = gtk::Label::new(Some("DSD Support:"));
    lbl_dsd.set_xalign(0.0);
    lbl_dsd.add_css_class("title-sm");
    box_.append(&lbl_dsd);

    let dsd_str = if dsd.is_empty() {
        "None detected (PCM only)".to_string()
    } else {
        format!("Native DSD ({})", dsd.join(", "))
    };
    let dsd_val = gtk::Label::new(Some(&dsd_str));
    dsd_val.set_xalign(0.0);
    dsd_val.set_wrap(true);
    if dsd.is_empty() {
        dsd_val.add_css_class("sub-text");
    } else {
        dsd_val.add_css_class("sub-green");
    }
    box_.append(&dsd_val);

    let close = gtk::Button::with_label("Close");
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| dialog.close());
    }
    box_.append(&close);

    dialog.set_child(Some(&box_));
    dialog.present();
}
