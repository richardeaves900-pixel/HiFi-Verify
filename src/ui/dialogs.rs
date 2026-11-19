use crate::audit::Audit;
use crate::config::AppConfig;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn modal_window(
    parent: &gtk::ApplicationWindow,
    title: &str,
    width: i32,
    height: i32,
) -> (gtk::Window, gtk::Box) {
    let dialog = gtk::Window::builder()
        .title(title)
        .default_width(width)
        .default_height(height)
        .transient_for(parent)
        .modal(true)
        .build();

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 10);
    box_.set_margin_top(14);
    box_.set_margin_bottom(14);
    box_.set_margin_start(14);
    box_.set_margin_end(14);
    (dialog, box_)
}

fn close_button(dialog: &gtk::Window) -> gtk::Button {
    let close = gtk::Button::with_label("Close");
    close.add_css_class("btn-cyber");
    let dlg = dialog.clone();
    close.connect_clicked(move |_| dlg.close());
    close
}

fn make_label(text: &str, css: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(css);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

pub fn show_audit_dialog(parent: &gtk::ApplicationWindow, audit: &Audit) {
    let (dialog, box_) = modal_window(parent, "Full Audio Path Audit", 680, 440);

    let verdict_css = match audit.audio_path_status {
        "clear" => "sub-green",
        "altered" => "sub-red",
        "unknown" => "sub-amber",
        _ => "sub-text",
    };
    box_.append(&make_label(&audit.verdict, verdict_css));

    let mut report = String::new();
    report.push_str("=== Findings ===\n");
    for f in &audit.findings {
        report.push_str(&format!(
            "[{}] {}: {}\n",
            f.severity.to_ascii_uppercase(),
            f.check,
            f.detail
        ));
    }
    if !audit.fixes.is_empty() {
        report.push_str("\n=== Suggested Actions ===\n");
        for fix in &audit.fixes {
            report.push_str(&format!("• {}\n  {}\n", fix.label, fix.action));
        }
    }

    let text_view = gtk::TextView::new();
    text_view.buffer().set_text(&report);
    text_view.set_editable(false);
    text_view.set_monospace(true);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&text_view));
    box_.append(&scroll);
    box_.append(&close_button(&dialog));

    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_ramp_result_dialog(parent: &gtk::ApplicationWindow, result: &serde_json::Value) {
    let status = result["status"].as_str().unwrap_or("error");
    let detail = result["detail"].as_str().unwrap_or("").to_string();

    let (dialog, box_) = modal_window(parent, "Loopback Ramp Test Result", 600, 240);

    let (badge_text, badge_css) = match status {
        "pass" => (
            "PASS — Exact sample match at monitor capture point",
            "sub-green",
        ),
        "fail" => ("FAIL — Captured samples differ from reference", "sub-red"),
        "skipped" => (
            "SKIPPED — No loopback monitor source available",
            "sub-amber",
        ),
        "inconclusive" => ("INCONCLUSIVE — Could not establish match", "sub-amber"),
        _ => ("ERROR", "sub-red"),
    };
    box_.append(&make_label(badge_text, badge_css));
    box_.append(&make_label(&detail, "sub-text"));
    if let Some(monitor) = result["monitor"].as_str() {
        box_.append(&make_label(&format!("Monitor: {monitor}"), "sub-text"));
    }
    if let Some(wav) = result["wav"].as_str() {
        box_.append(&make_label(&format!("Reference WAV: {wav}"), "sub-text"));
    }
    box_.append(&close_button(&dialog));

    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_wireplumber_dialog(
    parent: &gtk::ApplicationWindow,
    rate: Option<u32>,
    quantum: Option<u32>,
) {
    let rule = crate::wireplumber::generate_rule("alsa_output.*", rate, quantum);
    let (dialog, box_) = modal_window(parent, "WirePlumber 0.5 Rule", 720, 380);

    box_.append(&make_label(
        &format!("Save as {}", crate::wireplumber::RULE_SAVE_PATH),
        "sub-text",
    ));

    let text_view = gtk::TextView::new();
    text_view.buffer().set_text(&rule);
    text_view.set_editable(false);
    text_view.set_monospace(true);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&text_view));
    box_.append(&scroll);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let copy_btn = gtk::Button::with_label("Copy to Clipboard");
    copy_btn.add_css_class("btn-app-green");
    {
        let rule_copy = rule.clone();
        let copy_btn_ref = copy_btn.clone();
        copy_btn.connect_clicked(move |_| {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&rule_copy);
                copy_btn_ref.set_label("Copied!");
            }
        });
    }
    actions.append(&copy_btn);
    actions.append(&close_button(&dialog));
    box_.append(&actions);

    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_network_dialog(parent: &gtk::ApplicationWindow, devices: Vec<String>) {
    let (dialog, box_) = modal_window(parent, "Network Streamers (UPnP / SSDP)", 460, 300);

    box_.append(&make_label("Discovered Media Renderers:", "title-sm"));

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);

    let list_box = gtk::ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::None);

    if devices.is_empty() {
        list_box.append(&make_label(
            "No devices found. Ensure your streamer is on the same subnet.",
            "sub-text",
        ));
    } else {
        for dev in devices {
            let row = make_label(&dev, "val-purple");
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            list_box.append(&row);
        }
    }

    scroll.set_child(Some(&list_box));
    box_.append(&scroll);
    box_.append(&close_button(&dialog));

    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_history_dialog(parent: &gtk::ApplicationWindow, summary: &str) {
    let (dialog, box_) = modal_window(parent, "XRUN History (Last 50)", 540, 400);

    let text_view = gtk::TextView::new();
    text_view.buffer().set_text(summary);
    text_view.set_editable(false);
    text_view.set_monospace(true);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&text_view));
    box_.append(&scroll);
    box_.append(&close_button(&dialog));

    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_dac_dialog(
    parent: &gtk::ApplicationWindow,
    rates: Vec<String>,
    formats: Vec<String>,
    dsd: Vec<String>,
) {
    let (dialog, box_) = modal_window(parent, "DAC Hardware Capabilities", 520, 360);

    box_.append(&make_label(
        "Supported Sample Rates (Playback):",
        "title-sm",
    ));
    let rate_str = if rates.is_empty() {
        "Not detected".to_string()
    } else {
        rates.join(", ")
    };
    box_.append(&make_label(&rate_str, "val-cyan"));

    box_.append(&make_label("Supported PCM Formats:", "title-sm"));
    let fmt_str = if formats.is_empty() {
        "Not detected".to_string()
    } else {
        formats.join(", ")
    };
    box_.append(&make_label(&fmt_str, "val-purple"));

    box_.append(&make_label("DSD Support:", "title-sm"));
    let dsd_str = if dsd.is_empty() {
        "None detected (PCM only)".to_string()
    } else {
        format!("Native DSD ({})", dsd.join(", "))
    };
    box_.append(&make_label(
        &dsd_str,
        if dsd.is_empty() {
            "sub-text"
        } else {
            "sub-green"
        },
    ));

    box_.append(&close_button(&dialog));
    dialog.set_child(Some(&box_));
    dialog.present();
}

pub fn show_shortcuts_dialog(
    parent: &gtk::ApplicationWindow,
    config: Rc<RefCell<AppConfig>>,
    on_saved: impl Fn(&AppConfig) + 'static,
) {
    let (dialog, box_) = modal_window(parent, "Configurable Tool Shortcuts", 540, 360);

    box_.append(&make_label(
        "Leave any command blank to auto-detect installed desktop tools. Saved to ~/.config/hifi-verify/config.json.",
        "sub-text",
    ));

    let current = config.borrow().clone();

    let add_field = |parent_box: &gtk::Box, title: &str, placeholder: &str, value: &str| {
        parent_box.append(&make_label(title, "title-sm"));
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(placeholder));
        entry.set_text(value);
        parent_box.append(&entry);
        entry
    };

    let e_player = add_field(
        &box_,
        "Music Player (clicking Player/Source card):",
        "Auto-detect (e.g. strawberry, elisa, flatpak run ...)",
        &current.player_cmd,
    );
    let e_audio = add_field(
        &box_,
        "Audio Settings Mixer:",
        "Auto-detect (kcmshell6 kcm_pulseaudio / gnome-control-center sound / pavucontrol)",
        &current.audio_settings_cmd,
    );
    let e_dsp = add_field(
        &box_,
        "DSP / Equalizer App:",
        "Auto-detect (easyeffects / jamesdsp)",
        &current.dsp_cmd,
    );
    let e_patchbay = add_field(
        &box_,
        "PipeWire Patchbay / Graph:",
        "Auto-detect (qpwgraph / helvum / carla)",
        &current.patchbay_cmd,
    );

    let status_lbl = make_label("", "sub-text");
    box_.append(&status_lbl);

    let btn_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let btn_save = gtk::Button::with_label("Save Shortcuts");
    btn_save.add_css_class("btn-app-green");
    let btn_reset = gtk::Button::with_label("Reset to Auto");
    btn_reset.add_css_class("btn-cyber");

    {
        let e_player = e_player.clone();
        let e_audio = e_audio.clone();
        let e_dsp = e_dsp.clone();
        let e_patchbay = e_patchbay.clone();
        btn_reset.connect_clicked(move |_| {
            e_player.set_text("");
            e_audio.set_text("");
            e_dsp.set_text("");
            e_patchbay.set_text("");
        });
    }

    {
        let dlg = dialog.clone();
        let status_lbl = status_lbl.clone();
        btn_save.connect_clicked(move |_| {
            let updated = AppConfig {
                player_cmd: e_player.text().trim().to_string(),
                audio_settings_cmd: e_audio.text().trim().to_string(),
                dsp_cmd: e_dsp.text().trim().to_string(),
                patchbay_cmd: e_patchbay.text().trim().to_string(),
            };
            match updated.save() {
                Ok(()) => {
                    *config.borrow_mut() = updated.clone();
                    on_saved(&updated);
                    dlg.close();
                }
                Err(err) => {
                    status_lbl.set_text(&format!("Failed to save config: {err}"));
                    status_lbl.add_css_class("sub-red");
                }
            }
        });
    }

    btn_row.append(&btn_save);
    btn_row.append(&btn_reset);
    btn_row.append(&close_button(&dialog));
    box_.append(&btn_row);

    dialog.set_child(Some(&box_));
    dialog.present();
}
