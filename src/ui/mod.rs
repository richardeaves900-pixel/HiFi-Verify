mod dialogs;
mod draw;
mod worker;

use crate::audit::Audit;
use crate::cava::CavaController;
use crate::config::{AppConfig, ShortcutKind};
use crate::model::{format_rate_khz, PathStatus, Sink};
use crate::tray::{HiFiTray, TrayAction};
use crate::ui::draw::Anim;
use crate::ui::worker::Snapshot;
use crate::vu::VuMeter;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

const CSS: &str = include_str!("style.css");

pub enum UiEvent {
    Snapshot(Box<Snapshot>),
    AuditResult(Box<Audit>),
    RampResult(serde_json::Value),
    WirePlumberRule(Option<u32>, Option<u32>),
    NetworkDevices(Vec<String>),
    DacCaps(Vec<String>, Vec<String>, Vec<String>),
    HistorySummary(String),
    RateFeedback(String),
}

struct Ui {
    node_box: gtk::Box,
    combo: gtk::ComboBoxText,
    load_sub: gtk::Label,
    codec_lbl: gtk::Label,
    codec_sub: gtk::Label,
    quant_lbl: gtk::Label,
    lat_lbl: gtk::Label,
    rate_lbl: gtk::Label,
    rate_sub: gtk::Label,
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
    tray_rx: Option<Receiver<TrayAction>>,
    tray_handle: Option<ksni::blocking::Handle<HiFiTray>>,
) {
    let app = gtk::Application::builder()
        .application_id(if std::env::var_os("HIFI_VERIFY_DEV").is_some() {
            "org.cachyos.HiFiVerify.Dev"
        } else {
            "org.cachyos.HiFiVerify"
        })
        .build();

    let tray_rx_shared = Arc::new(std::sync::Mutex::new(tray_rx));
    let tray_handle_shared = Arc::new(std::sync::Mutex::new(tray_handle));

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
    tray_rx: Option<Receiver<TrayAction>>,
    tray_handle: Option<ksni::blocking::Handle<HiFiTray>>,
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

    let config = Rc::new(RefCell::new(AppConfig::load()));
    let worker_stop = Arc::new(AtomicBool::new(false));
    let cava = CavaController::new();
    let vu_meter = VuMeter::new();
    let (event_tx, event_rx) = channel::<UiEvent>();

    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("HiFi Verify")
        .default_width(740)
        .default_height(445)
        .build();

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
                    crate::host::host_popen("wpctl", &["set-default", &id]);
                }
            }
        });
    }

    let btn_net = gtk::Button::with_label("🌐 Scan Network");
    btn_net.add_css_class("btn-cyber");
    {
        let tx = event_tx.clone();
        let btn = btn_net.clone();
        btn_net.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let devices = crate::network::scan_network();
                let _ = tx.send(UiEvent::NetworkDevices(devices));
            });
        });
    }
    tools_row.append(&btn_net);

    let btn_dac = gtk::Button::with_label("🔍 DAC Caps");
    btn_dac.add_css_class("btn-cyber");
    {
        let tx = event_tx.clone();
        let btn = btn_dac.clone();
        btn_dac.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let (rates, formats, dsd) = crate::dac::get_dac_capabilities();
                let _ = tx.send(UiEvent::DacCaps(rates, formats, dsd));
            });
        });
    }
    tools_row.append(&btn_dac);

    // PipeWire Sink Volume Control (initialized asynchronously from worker snapshot)
    let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let vol_icon = gtk::Label::new(Some("🔊"));
    let vol_slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    vol_slider.set_draw_value(false);
    vol_slider.set_size_request(92, 24);
    vol_slider.set_valign(gtk::Align::Center);
    vol_slider.set_value(1.0);
    vol_slider.set_tooltip_text(Some("PipeWire sink volume (may use software gain)"));

    let updating_volume = Rc::new(Cell::new(false));
    let (volume_tx, volume_rx) = channel::<f64>();
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

    // Media Controls (off-thread D-Bus calls)
    let media_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    media_box.add_css_class("btn-cyber");
    for (icon, tip, cmd) in [
        ("⏮", "Previous Track", "Previous"),
        ("⏯", "Play / Pause", "PlayPause"),
        ("⏭", "Next Track", "Next"),
    ] {
        let btn = gtk::Button::with_label(icon);
        btn.set_tooltip_text(Some(tip));
        btn.connect_clicked(move |_| {
            std::thread::spawn(move || crate::telemetry::send_mpris_command(cmd));
        });
        media_box.append(&btn);
    }
    top_first.append(&media_box);

    // Now Playing Text
    let now_lbl = label("♫ NOW PLAYING: Detecting...", "chain-text", true);
    now_lbl.set_margin_start(4);
    now_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    now_lbl.set_hexpand(true);
    now_lbl.set_max_width_chars(42);
    top_first.append(&now_lbl);

    // Configurable desktop-agnostic tool shortcuts
    for (text, cls, kind) in [
        ("⚙ DSP / EQ", "btn-app-green", ShortcutKind::Dsp),
        (
            "🔊 Audio Settings",
            "btn-cyber",
            ShortcutKind::AudioSettings,
        ),
        ("🔗 Patchbay", "btn-cyber", ShortcutKind::Patchbay),
    ] {
        let b = gtk::Button::with_label(text);
        b.add_css_class(cls);
        let cfg = config.clone();
        b.connect_clicked(move |_| {
            cfg.borrow().launch_async(kind);
        });
        top_links.append(&b);
    }

    let btn_shortcuts = gtk::Button::with_label("✎ Configure Shortcuts");
    btn_shortcuts.add_css_class("btn-cyber");
    top_links.append(&btn_shortcuts);

    topbar.append(&top_first);
    root.append(&topbar);

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

    // Temporary PipeWire runtime rate controls
    let rate_tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let btn_match_rate = gtk::Button::with_label("Match stream rate (temporary)");
    btn_match_rate.add_css_class("btn-cyber");
    btn_match_rate.set_tooltip_text(Some(
        "Requests the playing PCM rate for the whole PipeWire graph. May interrupt other audio. Not persistent.",
    ));
    let btn_auto_rate = gtk::Button::with_label("Release forced rate / Auto");
    btn_auto_rate.add_css_class("btn-cyber");
    btn_auto_rate.set_tooltip_text(Some(
        "Remove a temporary rate override and return PipeWire to its own policy.",
    ));
    let rate_feedback = gtk::Label::new(Some("No rate override requested by HiFi Verify"));
    rate_feedback.add_css_class("sub-text");
    rate_feedback.set_ellipsize(gtk::pango::EllipsizeMode::End);
    rate_feedback.set_xalign(0.0);

    {
        let tx = event_tx.clone();
        let b1 = btn_match_rate.clone();
        let b2 = btn_auto_rate.clone();
        btn_match_rate.connect_clicked(move |_| {
            b1.set_sensitive(false);
            b2.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let graph = crate::telemetry::PwGraph::load();
                let stream = crate::telemetry::detect_active_stream(&graph);
                let rows = crate::pwtop::parse_pw_top()
                    .map(|s| s.rows)
                    .unwrap_or_default();
                let rate = stream
                    .as_ref()
                    .and_then(|s| s.sr)
                    .or_else(|| graph.pcm_for_route(&rows).map(|p| p.rate));
                let message = match rate {
                    Some(rate)
                        if [
                            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
                        ]
                        .contains(&rate) =>
                    {
                        let value = rate.to_string();
                        match crate::host::host_cmd(
                            &[
                                "pw-metadata",
                                "-n",
                                "settings",
                                "0",
                                "clock.force-rate",
                                &value,
                            ],
                            Duration::from_secs(3),
                        ) {
                            Ok(output) if output.status.success() => {
                                format!("Requested {} Hz globally; verify DAC reading", rate)
                            }
                            Ok(output) => format!(
                                "Rate request failed: {}",
                                String::from_utf8_lossy(&output.stderr).trim()
                            ),
                            Err(err) => format!("Rate request failed: {err}"),
                        }
                    }
                    Some(rate) => format!("Unsupported automatic rate request: {rate} Hz"),
                    None => "Active linked stream rate unavailable; no settings changed".into(),
                };
                let _ = tx.send(UiEvent::RateFeedback(message));
            });
        });
    }
    {
        let tx = event_tx.clone();
        let b1 = btn_match_rate.clone();
        let b2 = btn_auto_rate.clone();
        btn_auto_rate.connect_clicked(move |_| {
            b1.set_sensitive(false);
            b2.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let msg = match crate::host::host_cmd(
                    &[
                        "pw-metadata",
                        "-n",
                        "settings",
                        "0",
                        "clock.force-rate",
                        "0",
                    ],
                    Duration::from_secs(3),
                ) {
                    Ok(output) if output.status.success() => {
                        "Removed rate override; PipeWire auto policy restored".to_string()
                    }
                    Ok(output) => format!(
                        "Could not release rate override: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                    Err(err) => format!("Could not release rate override: {err}"),
                };
                let _ = tx.send(UiEvent::RateFeedback(msg));
            });
        });
    }

    rate_tools.append(&btn_match_rate);
    rate_tools.append(&btn_auto_rate);
    tools_content.append(&rate_tools);
    tools_content.append(&rate_feedback);
    tools_expander.set_child(Some(&tools_content));
    root.append(&tools_expander);

    // ---- Cards -----------------------------------------------------------
    let cards = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let cards_row_one = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    cards_row_one.set_homogeneous(true);
    let cards_row_two = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    cards_row_two.set_homogeneous(true);

    let c1 = card(&cards_row_one, &["card-green"]);
    c1.append(&label("DSP LOAD (W/Q)", "title-sm", false));
    let gauge_area = gtk::DrawingArea::new();
    gauge_area.set_content_height(28);
    c1.append(&gauge_area);
    let load_sub = label("Standby", "sub-text", false);
    c1.append(&load_sub);

    let c_src = card(&cards_row_one, &["card-purple", "card-interactive"]);
    let update_src_tooltip = {
        let c_src = c_src.clone();
        move |cfg: &AppConfig| {
            let tip = if cfg.player_cmd.trim().is_empty() {
                "Click to launch your installed music player (configurable in Devices & tools)"
                    .to_string()
            } else {
                format!("Click to launch: {}", cfg.player_cmd.trim())
            };
            c_src.set_tooltip_text(Some(&tip));
        }
    };
    update_src_tooltip(&config.borrow());
    c_src.set_cursor_from_name(Some("pointer"));
    {
        let cfg = config.clone();
        let gesture = gtk::GestureClick::new();
        gesture.connect_pressed(move |_, _, _, _| {
            cfg.borrow().launch_async(ShortcutKind::Player);
        });
        c_src.add_controller(gesture);
    }

    c_src.append(&label("PLAYER / SOURCE FORMAT", "title-sm", false));
    let codec_lbl = label("IDLE", "val-purple", false);
    codec_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    codec_lbl.set_max_width_chars(24);
    c_src.append(&codec_lbl);
    let codec_sub = label("No active playback", "sub-purple", false);
    codec_sub.set_wrap(true);
    codec_sub.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    codec_sub.set_max_width_chars(26);
    c_src.append(&codec_sub);

    let c2 = card(&cards_row_one, &["card-cyan"]);
    c2.append(&label("BUFFER (QUANT)", "title-sm", false));
    let quant_lbl = label("—", "val-cyan", false);
    c2.append(&quant_lbl);
    let lat_lbl = label("samples • —", "sub-text", false);
    c2.append(&lat_lbl);

    let c3 = card(&cards_row_two, &["card-gold"]);
    c3.append(&label("ACTIVE OUTPUT RATE", "title-sm", false));
    let rate_lbl = label("—", "val-gold", false);
    c3.append(&rate_lbl);
    let rate_sub = label("Standby", "sub-text", false);
    c3.append(&rate_sub);

    let c4 = card(&cards_row_two, &["card-green"]);
    c4.append(&label("XRUNS (SESSION)", "title-sm", false));
    let err_lbl = label("0", "val-green", false);
    c4.append(&err_lbl);
    let err_sub = label("No Dropped Frames", "sub-green", false);
    c4.append(&err_sub);

    let c5 = card(&cards_row_two, &["card-cyan", "card-interactive"]);
    let update_patchbay_tooltip = {
        let c5 = c5.clone();
        move |cfg: &AppConfig| {
            let tip = if cfg.patchbay_cmd.trim().is_empty() {
                "Click to open PipeWire patchbay (auto-detects qpwgraph / helvum / carla)"
                    .to_string()
            } else {
                format!("Click to launch: {}", cfg.patchbay_cmd.trim())
            };
            c5.set_tooltip_text(Some(&tip));
        }
    };
    update_patchbay_tooltip(&config.borrow());
    c5.set_cursor_from_name(Some("pointer"));
    {
        let cfg = config.clone();
        let gesture = gtk::GestureClick::new();
        gesture.connect_pressed(move |_, _, _, _| {
            cfg.borrow().launch_async(ShortcutKind::Patchbay);
        });
        c5.add_controller(gesture);
    }

    c5.append(&label("DAC BUS FORMAT & VU", "title-sm", false));
    let fmt_lbl = label("—", "val-cyan", false);
    c5.append(&fmt_lbl);
    let vu_area = gtk::DrawingArea::new();
    vu_area.set_content_height(18);
    vu_area.set_tooltip_text(Some(
        "Real-time stereo sink monitor VU meter (-60 to 0 dBFS)",
    ));
    c5.append(&vu_area);

    // Wire Shortcuts button now that tooltip updaters exist.
    {
        let parent = win.clone();
        let cfg = config.clone();
        btn_shortcuts.connect_clicked(move |_| {
            let u1 = update_src_tooltip.clone();
            let u2 = update_patchbay_tooltip.clone();
            dialogs::show_shortcuts_dialog(&parent, cfg.clone(), move |updated| {
                u1(updated);
                u2(updated);
            });
        });
    }

    cards.append(&cards_row_one);
    cards.append(&cards_row_two);
    root.append(&cards);

    // ---- Audit strip ----------------------------------------------------
    let audit_bar = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    audit_bar.add_css_class("topbar-box");
    audit_bar.append(&label("AUDIT:", "title-sm", false));
    let audit_summary = label("Click Run Audit to check the chain", "sub-text", true);
    audit_summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    audit_summary.set_max_width_chars(44);
    audit_bar.append(&audit_summary);

    let btn_audit = gtk::Button::with_label("Run Audit");
    btn_audit.add_css_class("btn-cyber");
    let btn_audit_details = gtk::Button::with_label("Details");
    btn_audit_details.add_css_class("btn-cyber");
    btn_audit_details.set_sensitive(false);

    let btn_ramp = gtk::Button::with_label("Loopback Ramp Test");
    btn_ramp.add_css_class("btn-cyber");
    let btn_hist = gtk::Button::with_label("📋 XRUN History");
    btn_hist.add_css_class("btn-cyber");
    let btn_wp = gtk::Button::with_label("WirePlumber Rule");
    btn_wp.add_css_class("btn-cyber");

    audit_bar.append(&btn_audit);
    audit_bar.append(&btn_audit_details);
    extra_tools.append(&btn_ramp);
    extra_tools.append(&btn_hist);
    extra_tools.append(&btn_wp);
    root.append(&audit_bar);

    let last_audit: Rc<RefCell<Option<Audit>>> = Rc::new(RefCell::new(None));
    {
        let parent = win.clone();
        let last_audit = last_audit.clone();
        btn_audit_details.connect_clicked(move |_| {
            if let Some(ref audit) = *last_audit.borrow() {
                dialogs::show_audit_dialog(&parent, audit);
            }
        });
    }
    {
        let tx = event_tx.clone();
        let btn = btn_audit.clone();
        btn_audit.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let audit = crate::audit::build_audit(false);
                let _ = tx.send(UiEvent::AuditResult(Box::new(audit)));
            });
        });
    }
    {
        let tx = event_tx.clone();
        let btn = btn_ramp.clone();
        btn_ramp.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = crate::ramp::run(48000);
                let _ = tx.send(UiEvent::RampResult(result));
            });
        });
    }
    {
        let tx = event_tx.clone();
        let btn = btn_hist.clone();
        btn_hist.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let summary = crate::history::get_history_summary();
                let _ = tx.send(UiEvent::HistorySummary(summary));
            });
        });
    }
    {
        let tx = event_tx.clone();
        let btn = btn_wp.clone();
        btn_wp.connect_clicked(move |_| {
            btn.set_sensitive(false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let a = crate::audit::build_audit(true);
                let _ = tx.send(UiEvent::WirePlumberRule(a.driver_rate, a.driver_quantum));
            });
        });
    }

    // ---- Body ------------------------------------------------------------
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    body.set_vexpand(false);

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
    scroll.set_min_content_height(240);
    scroll.set_max_content_height(320);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&node_box));
    left.append(&scroll);
    body.append(&left);

    let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
    right.add_css_class("card");
    right.set_size_request(230, -1);

    let toggle_btn = gtk::ToggleButton::with_label("SIGNAL PATH MATRIX");
    toggle_btn.add_css_class("btn-cyber");
    right.append(&toggle_btn);

    let spec_data = cava.bands();
    let spec_area = gtk::DrawingArea::new();
    spec_area.set_content_height(195);
    spec_area.set_vexpand(false);
    right.append(&spec_area);

    right.append(&label("LATENCY JITTER (us)", "title-sm", false));
    let scope_area = gtk::DrawingArea::new();
    scope_area.set_content_height(52);
    scope_area.set_vexpand(true);
    right.append(&scope_area);

    let btn_reset = gtk::Button::with_label("Reset XRUN Counters");
    btn_reset.set_tooltip_text(Some("Reset the session XRUN counter baseline to 0"));
    btn_reset.add_css_class("btn-cyber");
    right.append(&btn_reset);

    right.append(&label(
        "Clock policy: Auto (override in Tools)",
        "sub-text",
        false,
    ));

    body.append(&right);

    let details_expander = gtk::Expander::new(Some("More details: nodes & signal graphs"));
    details_expander.set_expanded(false);
    details_expander.set_child(Some(&body));
    root.append(&details_expander);

    let viewport = gtk::ScrolledWindow::new();
    viewport.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    viewport.set_child(Some(&root));
    win.set_child(Some(&viewport));

    // ---- Draw callbacks (registered exactly once each) ------------------
    let anim = Rc::new(RefCell::new(Anim::new()));

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
    {
        let a = anim.clone();
        scope_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::scope(cr, w as f64, h as f64, &aa.jitter);
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
        vu_area.set_draw_func(move |_, cr, w, h| {
            let aa = a.borrow();
            draw::vu(cr, w as f64, h as f64, &aa.vu);
        });
    }

    // Lazy cava lifecycle: run ONLY when both spectrum mode AND details panel are visible.
    let sync_cava = {
        let cava = cava.clone();
        let toggle = toggle_btn.clone();
        let exp = details_expander.clone();
        let win_ref = win.clone();
        move || {
            if win_ref.is_visible() && exp.is_expanded() && toggle.is_active() {
                cava.start();
            } else {
                cava.stop();
            }
        }
    };
    {
        let spec_area_clone = spec_area.clone();
        let sync = sync_cava.clone();
        toggle_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                btn.set_label("AUDIO SPECTRUM");
            } else {
                btn.set_label("SIGNAL PATH MATRIX");
            }
            sync();
            spec_area_clone.queue_draw();
        });
    }
    {
        let sync = sync_cava.clone();
        details_expander.connect_expanded_notify(move |_| {
            sync();
        });
    }

    // ---- Worker + Unified UI Event Drain --------------------------------
    worker::spawn_worker(event_tx, worker_stop.clone());

    let ui = Rc::new(Ui {
        node_box,
        combo,
        load_sub,
        codec_lbl,
        codec_sub,
        quant_lbl,
        lat_lbl,
        rate_lbl,
        rate_sub,
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
    let last_sinks: Rc<RefCell<Vec<Sink>>> = Rc::new(RefCell::new(Vec::new()));
    let is_playing_flag = Rc::new(Cell::new(false));

    // Single 100 ms event drain for ALL background workers + tray events.
    {
        let ui = ui.clone();
        let anim = anim.clone();
        let updating_combo = updating_combo.clone();
        let err_baseline = err_baseline.clone();
        let last_seen_err = last_seen_err.clone();
        let baseline_initialized = baseline_initialized.clone();
        let last_sinks = last_sinks.clone();
        let tray_handle_clone = tray_handle.clone();
        let updating_volume = updating_volume.clone();
        let win_clone = win.clone();
        let app_clone = app.clone();
        let vu_ctrl = vu_meter.clone();
        let cava_ctrl = cava.clone();
        let stop_flag = worker_stop.clone();
        let is_playing_flag = is_playing_flag.clone();
        let sync_cava_ev = sync_cava.clone();
        let last_audit_ev = last_audit.clone();

        glib::timeout_add_local(Duration::from_millis(100), move || {
            // Drain tray actions first.
            if let Some(ref rx) = tray_rx {
                while let Ok(action) = rx.try_recv() {
                    match action {
                        TrayAction::ToggleWindow => {
                            if win_clone.is_visible() {
                                win_clone.set_visible(false);
                                vu_ctrl.stop();
                                cava_ctrl.stop();
                            } else {
                                win_clone.present();
                                sync_cava_ev();
                                if is_playing_flag.get() {
                                    vu_ctrl.start();
                                }
                            }
                        }
                        TrayAction::Quit => {
                            stop_flag.store(true, Ordering::SeqCst);
                            vu_ctrl.stop();
                            cava_ctrl.stop();
                            app_clone.quit();
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            }

            while let Ok(ev) = event_rx.try_recv() {
                match ev {
                    UiEvent::Snapshot(snap) => {
                        if !baseline_initialized.get() {
                            err_baseline.set(snap.total_err);
                            baseline_initialized.set(true);
                        }
                        last_seen_err.set(snap.total_err);
                        let delta = snap.total_err.saturating_sub(err_baseline.get());

                        render(&ui, &anim, &updating_combo, &last_sinks, delta, &snap);

                        let playing = snap.stream.sr.is_some()
                            || snap.stream.codec.is_some()
                            || snap.stream.app.is_some();
                        is_playing_flag.set(playing);

                        // Passive real VU meter runs ONLY when visible AND playing.
                        if win_clone.is_visible() && playing {
                            vu_ctrl.start();
                        } else {
                            vu_ctrl.stop();
                        }

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
                            update_tray(th, &snap, delta);
                        }
                    }
                    UiEvent::AuditResult(audit) => {
                        btn_audit.set_sensitive(true);
                        btn_audit_details.set_sensitive(true);
                        let warns = audit
                            .findings
                            .iter()
                            .filter(|f| f.severity == "warn")
                            .count();
                        let summary_text = if warns == 0 {
                            audit.verdict.clone()
                        } else {
                            let first = audit
                                .findings
                                .iter()
                                .find(|f| f.severity == "warn")
                                .map(|f| f.detail.as_str())
                                .unwrap_or("");
                            format!("{} — {} warning(s): {}", audit.verdict, warns, first)
                        };
                        audit_summary.set_text(&summary_text);
                        audit_summary.set_tooltip_text(Some(&summary_text));
                        *last_audit_ev.borrow_mut() = Some(*audit);
                    }
                    UiEvent::RampResult(result) => {
                        btn_ramp.set_sensitive(true);
                        dialogs::show_ramp_result_dialog(&win_clone, &result);
                    }
                    UiEvent::WirePlumberRule(rate, quantum) => {
                        btn_wp.set_sensitive(true);
                        dialogs::show_wireplumber_dialog(&win_clone, rate, quantum);
                    }
                    UiEvent::NetworkDevices(devices) => {
                        btn_net.set_sensitive(true);
                        dialogs::show_network_dialog(&win_clone, devices);
                    }
                    UiEvent::DacCaps(rates, formats, dsd) => {
                        btn_dac.set_sensitive(true);
                        dialogs::show_dac_dialog(&win_clone, rates, formats, dsd);
                    }
                    UiEvent::HistorySummary(summary) => {
                        btn_hist.set_sensitive(true);
                        dialogs::show_history_dialog(&win_clone, &summary);
                    }
                    UiEvent::RateFeedback(msg) => {
                        btn_match_rate.set_sensitive(true);
                        btn_auto_rate.set_sensitive(true);
                        rate_feedback.set_text(&msg);
                        rate_feedback.set_tooltip_text(Some(&msg));
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Visibility-aware 33 ms (~30 FPS) animation tick for real VU meter & active spectrum.
    {
        let ui = ui.clone();
        let anim = anim.clone();
        let vu_ctrl = vu_meter.clone();
        let win_ref = win.clone();
        let exp_ref = details_expander.clone();
        let toggle_ref = toggle_btn.clone();
        glib::timeout_add_local(Duration::from_millis(33), move || {
            if !win_ref.is_visible() {
                return glib::ControlFlow::Continue;
            }
            vu_ctrl.decay_tick();
            let lv = vu_ctrl.snapshot();
            let needs_vu = lv.active
                || lv.left > 0.001
                || lv.right > 0.001
                || lv.peak_l > 0.001
                || lv.peak_r > 0.001;
            {
                let mut a = anim.borrow_mut();
                a.vu = lv;
            }
            if needs_vu {
                ui.vu_area.queue_draw();
            }
            if exp_ref.is_expanded() && toggle_ref.is_active() {
                ui.spec_area.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }

    // Reset session XRUN display baseline.
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

    // Window close behavior: hide to tray ONLY when a tray icon is actually available;
    // otherwise cleanly stop workers and exit so the app is never orphaned invisibly.
    let has_tray = tray_handle.is_some();
    win.set_hide_on_close(has_tray);
    {
        let vu_ctrl = vu_meter.clone();
        let cava_ctrl = cava.clone();
        let stop_flag = worker_stop.clone();
        win.connect_close_request(move |win| {
            vu_ctrl.stop();
            cava_ctrl.stop();
            if has_tray {
                win.set_visible(false);
                glib::Propagation::Stop
            } else {
                stop_flag.store(true, Ordering::SeqCst);
                glib::Propagation::Proceed
            }
        });
    }
    win.present();
}

fn update_tray(th: &ksni::blocking::Handle<HiFiTray>, snap: &Snapshot, session_err: u64) {
    let path_status = snap
        .path
        .as_ref()
        .map(|p| p.status)
        .unwrap_or(PathStatus::Idle);
    let status_label = snap
        .path
        .as_ref()
        .map(|p| p.explanation.clone())
        .unwrap_or_else(|| "Standby".into());

    let mut detected_processing = snap.processing.clone();
    if let Some(ref source) = snap.stream.source_processing {
        detected_processing.push(source.clone());
    }
    let observed_pcm = snap.path.as_ref().and_then(|p| p.pcm.as_ref());

    let app_name = snap.stream.app.clone().unwrap_or_else(|| "Idle".into());
    let track_name = snap.stream.track.clone().unwrap_or_default();
    let codec = snap
        .stream
        .codec
        .as_deref()
        .unwrap_or("Source codec not exposed");
    let stream_info = match (snap.stream.sr, snap.stream.bps) {
        (Some(rate), Some(bits)) => {
            format!("{} · {}-bit / {}", codec, bits, format_rate_khz(rate))
        }
        (Some(rate), None) => format!("{} · {}", codec, format_rate_khz(rate)),
        _ => observed_pcm
            .map(|p| format!("{} | PCM {} @ {}", codec, p.format, format_rate_khz(p.rate)))
            .unwrap_or_else(|| codec.to_string()),
    };
    let pipewire_info = match (snap.driver_rate, snap.driver_quantum) {
        (Some(rate), Some(q)) => format!("{} · Q{}", format_rate_khz(rate), q),
        (Some(rate), None) => format!("{} · quantum unknown", format_rate_khz(rate)),
        (None, Some(q)) => format!("rate unknown · Q{}", q),
        _ => "Idle / Unknown".into(),
    };
    let hw_info = snap
        .dac_hw
        .as_ref()
        .map(|hw| format!("{} @ {}", hw.format, format_rate_khz(hw.rate)))
        .unwrap_or_else(|| "No open playback PCM".into());

    let route_dsp = snap
        .path
        .as_ref()
        .map(|p| p.route_dsp.as_slice())
        .unwrap_or(&[]);
    let dsp_info = if !route_dsp.is_empty() {
        format!("IN ROUTE: {}", route_dsp.join(" → "))
    } else if !detected_processing.is_empty() {
        format!("Running (not in route): {}", detected_processing.join(", "))
    } else if snap.stream.source_processing_checked {
        "Player gain unity / normalization off (reported)".into()
    } else {
        "No PipeWire DSP detected".into()
    };

    let dac_rate_hz = snap.dac_hw.as_ref().map(|h| h.rate).or(snap.driver_rate);
    let dac_format = snap
        .dac_hw
        .as_ref()
        .map(|h| h.format.clone())
        .or_else(|| observed_pcm.map(|p| p.format.clone()))
        .unwrap_or_default();

    let xrun_info = format!("{} session ({} total)", session_err, snap.total_err);

    let _ = th.update(move |t| {
        t.status = path_status;
        t.status_label = status_label;
        t.dac_rate_hz = dac_rate_hz;
        t.dac_format = dac_format;
        t.app_name = app_name;
        t.track_name = track_name;
        t.stream_info = stream_info;
        t.hw_info = hw_info;
        t.dsp_info = dsp_info;
        t.pipewire_info = pipewire_info;
        t.xrun_info = xrun_info;
    });
}

fn render(
    ui: &Ui,
    anim: &RefCell<Anim>,
    updating_combo: &Cell<bool>,
    last_sinks: &RefCell<Vec<Sink>>,
    err_delta: u64,
    snap: &Snapshot,
) {
    // Reuse existing GTK labels for the node list.
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
    while let Some(widget) = child {
        child = widget.next_sibling();
        ui.node_box.remove(&widget);
    }

    // Sink combo — rebuild ONLY when the set of sinks actually changed.
    if *last_sinks.borrow() != snap.sinks {
        *last_sinks.borrow_mut() = snap.sinks.clone();
        let current = ui.combo.active_id().map(|s| s.to_string());
        updating_combo.set(true);
        ui.combo.remove_all();
        ui.combo.append(Some("default"), "Default PipeWire Sink");
        for s in &snap.sinks {
            ui.combo.append(Some(&s.id), &s.description);
        }
        let restore = current
            .filter(|c| c == "default" || snap.sinks.iter().any(|s| s.id == *c))
            .unwrap_or_else(|| "default".to_string());
        ui.combo.set_active_id(Some(&restore));
        updating_combo.set(false);
    }

    let app_name = snap.stream.app.as_deref().filter(|s| !s.is_empty());
    let codec = snap.stream.codec.as_deref();
    ui.codec_lbl.set_text(&match (app_name, codec) {
        (Some(app), Some(fmt)) => format!("{} · {}", app, fmt),
        (Some(app), None) => app.to_string(),
        (None, Some(fmt)) => fmt.to_string(),
        (None, None) => "IDLE".to_string(),
    });

    let app = app_name.unwrap_or("Standby");
    let codec_name = codec.unwrap_or("Source codec not exposed");
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

    if app_name.is_some() {
        let sub = format!("{} | {} → {}", source, stream_pcm, dac_pcm);
        let brief_source = if codec.is_some() {
            source
        } else {
            "Source metadata unexposed".to_string()
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
    } else {
        ui.codec_sub.set_text("No active playback");
        ui.codec_sub.set_tooltip_text(None);
    }

    if let Some(q) = snap.driver_quantum {
        ui.quant_lbl.set_text(&q.to_string());
    } else {
        ui.quant_lbl.set_text("—");
    }
    if let (Some(q), Some(r)) = (snap.driver_quantum, snap.driver_rate) {
        ui.lat_lbl
            .set_text(&format!("samples • {:.1}ms", q as f64 / r as f64 * 1000.0));
    } else {
        ui.lat_lbl.set_text("samples • —");
    }

    if let Some(r) = snap.driver_rate {
        ui.rate_lbl.set_text(&format_rate_khz(r));
        ui.rate_sub.set_text(if r >= 88_200 {
            "Hi-Res PCM"
        } else {
            "Standard PCM"
        });
    } else {
        ui.rate_lbl.set_text("—");
        ui.rate_sub.set_text("Standby");
    }

    ui.err_lbl.set_text(&err_delta.to_string());
    if err_delta > 0 {
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
            .unwrap_or("—"),
    );

    if app_name.is_some() {
        let track = snap.stream.track.as_deref().unwrap_or("Active Stream");
        ui.now_lbl.set_text(&format!("♫ [{}] {}", app, track));
    } else {
        ui.now_lbl.set_text("♫ STANDBY — No active audio stream");
    }

    // Active processing notification (sent off main thread).
    let route_dsp = snap
        .path
        .as_ref()
        .map(|p| p.route_dsp.as_slice())
        .unwrap_or(&[]);
    let active_processing = !route_dsp.is_empty() || snap.stream.source_processing.is_some();
    {
        let mut a = anim.borrow_mut();
        if active_processing {
            if !a.is_processing {
                a.is_processing = true;
                let reason = snap.stream.source_processing.clone().unwrap_or_else(|| {
                    format!("Active in playback route: {}", route_dsp.join(" → "))
                });
                std::thread::spawn(move || {
                    let _ = notify_rust::Notification::new()
                        .summary("HiFi Verify: Processing in playback path")
                        .body(&reason)
                        .icon("dialog-warning")
                        .show();
                });
            }
        } else {
            a.is_processing = false;
        }
    }

    // Feed live telemetry into canvas state (only active running nodes for DSP load).
    let max_wq = snap
        .rows
        .iter()
        .filter(|r| r.status == 'R')
        .map(|r| r.wq)
        .fold(0.0f32, f32::max) as f64;
    ui.load_sub.set_text(if max_wq >= 85.0 {
        "High Load / Overrun Risk"
    } else if max_wq >= 50.0 {
        "Moderate Load"
    } else if max_wq > 0.0 {
        "Minimal Latency"
    } else {
        "Standby"
    });

    {
        let mut a = anim.borrow_mut();
        a.dsp_load = max_wq;

        // Only push jitter point when pw-top delivered a fresh reading (every ~2s).
        if snap.pwtop_updated {
            if let Some(wait) = snap.wait_us {
                let norm = ((wait / 1000.0) * 100.0).clamp(0.0, 100.0);
                a.jitter.remove(0);
                a.jitter.push(norm);
            }
        }

        a.matrix_src_app = snap.stream.app.clone().unwrap_or_else(|| "Standby".into());
        a.matrix_src_codec = snap
            .stream
            .codec
            .clone()
            .unwrap_or_else(|| "Unknown".into());

        let mut processing = route_dsp.to_vec();
        if let Some(ref source) = snap.stream.source_processing {
            processing.push(source.clone());
        }
        a.matrix_dsp = if !processing.is_empty() {
            format!("ACTIVE: {}", processing.join(" → "))
        } else if !snap.processing.is_empty() {
            format!(
                "Bypassed ({} running elsewhere)",
                snap.processing.join(", ")
            )
        } else if snap.stream.source_processing_checked {
            "Player gain unity / norm off (reported)".to_string()
        } else {
            "No PipeWire DSP in route".to_string()
        };

        // Strictly use hardware DAC params for the DAC BUS box (never graph fallback).
        if let Some(ref hw) = snap.dac_hw {
            a.matrix_hw_fmt = hw.format.clone();
            a.matrix_rate = hw.rate as f64 / 1000.0;
        } else {
            a.matrix_hw_fmt = "---".into();
            a.matrix_rate = 0.0;
        }

        a.path_status = snap
            .path
            .as_ref()
            .map(|p| p.status)
            .unwrap_or(PathStatus::Idle);
        a.path_reason = snap
            .path
            .as_ref()
            .map(|p| p.explanation.clone())
            .unwrap_or_else(|| "No active playback".into());
    }

    // Queue redraws on data change instead of spamming 30 FPS when static.
    ui.gauge_area.queue_draw();
    ui.scope_area.queue_draw();
    ui.spec_area.queue_draw();
}

fn label(text: &str, css: &str, hexpand: bool) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(css);
    l.set_xalign(0.0);
    l.set_hexpand(hexpand);
    l
}

fn card(container: &gtk::Box, classes: &[&str]) -> gtk::Box {
    let c = gtk::Box::new(gtk::Orientation::Vertical, 2);
    c.add_css_class("card");
    for cls in classes {
        c.add_css_class(cls);
    }
    container.append(&c);
    c
}
