use crate::model::PathStatus;
use crate::vu::VuLevels;
use cairo::Context;

pub const JITTER_SAMPLES: usize = 30;

pub struct Anim {
    pub jitter: Vec<f64>,
    pub dsp_load: f64,
    pub vu: VuLevels,
    pub matrix_src_app: String,
    pub matrix_src_codec: String,
    pub matrix_dsp: String,
    pub matrix_hw_fmt: String,
    pub matrix_rate: f64,
    pub is_processing: bool,
    pub path_status: PathStatus,
    pub path_reason: String,
}

impl Anim {
    pub fn new() -> Self {
        Self {
            jitter: vec![0.0; JITTER_SAMPLES],
            dsp_load: 0.0,
            vu: VuLevels::default(),
            matrix_src_app: "Detecting...".into(),
            matrix_src_codec: "---".into(),
            matrix_dsp: "---".into(),
            matrix_hw_fmt: "---".into(),
            matrix_rate: 0.0,
            is_processing: false,
            path_status: PathStatus::Idle,
            path_reason: "No active playback".into(),
        }
    }
}

pub fn gauge(cr: &Context, w: f64, h: f64, load: f64) {
    let (cx, cy) = (w / 2.0, h - 5.0);
    let r = (w / 2.0 - 12.0).min(h - 8.0).max(8.0);
    cr.set_line_width(5.5);
    cr.set_source_rgba(0.1, 0.2, 0.18, 0.8);
    cr.arc(cx, cy, r, std::f64::consts::PI, 2.0 * std::f64::consts::PI);
    let _ = cr.stroke();

    let frac = (load / 100.0).clamp(0.0, 1.0);
    let (r_c, g_c, b_c) = if load >= 85.0 {
        (0.94, 0.27, 0.27)
    } else if load >= 50.0 {
        (0.96, 0.62, 0.04)
    } else {
        (0.06, 0.72, 0.5)
    };
    if frac > 0.0 {
        cr.set_source_rgb(r_c, g_c, b_c);
        cr.arc(
            cx,
            cy,
            r,
            std::f64::consts::PI,
            std::f64::consts::PI + frac * std::f64::consts::PI,
        );
        let _ = cr.stroke();
    }

    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size(14.0);
    cr.set_source_rgb(r_c, g_c, b_c);
    let txt = format!("{:.1}%", load);
    if let Ok(ext) = cr.text_extents(&txt) {
        cr.move_to(cx - ext.width() / 2.0, cy - 2.0);
        let _ = cr.show_text(&txt);
    }
}

/// Draw text clipped to the current panel width; never index UTF-8 by bytes.
fn fitted_text(cr: &Context, text: &str, width: f64) -> String {
    if width <= 0.0 {
        return String::new();
    }
    let fits = |s: &str| {
        cr.text_extents(s)
            .is_ok_and(|size| size.x_advance() <= width)
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for count in (0..chars.len()).rev() {
        let candidate = chars[..count].iter().collect::<String>() + "…";
        if fits(&candidate) {
            return candidate;
        }
    }
    String::new()
}

pub fn chain_matrix(cr: &Context, w: f64, h: f64, anim: &Anim) {
    if w < 55.0 || h < 95.0 {
        return;
    }
    cr.save().ok();
    cr.rectangle(0.0, 0.0, w, h);
    cr.clip();
    cr.set_source_rgba(0.03, 0.05, 0.07, 0.9);
    cr.paint().ok();

    let inset = 9.0;
    let gap = if h >= 200.0 { 9.0 } else { 5.0 };
    let status_room = if h >= 195.0 { 26.0 } else { 0.0 };
    let box_w = w - 2.0 * inset;
    let box_h = ((h - status_room - 4.0 * gap) / 3.0).max(25.0);
    let text_w = (box_w - 16.0).max(0.0);

    let src = format!("{} | {}", anim.matrix_src_app, anim.matrix_src_codec);
    let dsp = if anim.matrix_dsp.trim().is_empty() || anim.matrix_dsp == "---" {
        "No PipeWire DSP reported".to_string()
    } else {
        anim.matrix_dsp.clone()
    };
    let dac = if anim.matrix_rate > 0.0 && anim.matrix_hw_fmt != "---" {
        format!("{} @ {:.1}kHz", anim.matrix_hw_fmt, anim.matrix_rate)
    } else {
        "No open DAC PCM".to_string()
    };
    let steps = [
        ("SOURCE", src.as_str(), (0.6, 0.4, 1.0)),
        ("DSP / PROCESSING", dsp.as_str(), (1.0, 0.8, 0.4)),
        ("DAC BUS", dac.as_str(), (0.4, 0.8, 0.6)),
    ];
    for (i, (title, value, (r, g, b))) in steps.iter().enumerate() {
        let y = gap + i as f64 * (box_h + gap);
        cr.set_source_rgba(*r, *g, *b, 0.12);
        cr.rectangle(inset, y, box_w, box_h);
        cr.fill().ok();
        cr.set_source_rgba(*r, *g, *b, 0.85);
        cr.set_line_width(1.0);
        cr.rectangle(inset + 0.5, y + 0.5, box_w - 1.0, box_h - 1.0);
        cr.stroke().ok();
        cr.save().ok();
        cr.rectangle(inset + 3.0, y + 2.0, box_w - 6.0, box_h - 4.0);
        cr.clip();
        cr.select_font_face(
            "monospace",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Bold,
        );
        cr.set_font_size(9.0);
        cr.set_source_rgba(*r, *g, *b, 1.0);
        cr.move_to(inset + 8.0, y + 12.0);
        cr.show_text(&fitted_text(cr, title, text_w)).ok();
        cr.set_font_size(10.0);
        cr.set_source_rgb(0.91, 0.93, 0.96);
        cr.move_to(inset + 8.0, y + box_h - 8.0);
        cr.show_text(&fitted_text(cr, value, text_w)).ok();
        cr.restore().ok();
    }
    if status_room > 0.0 {
        cr.select_font_face(
            "monospace",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Bold,
        );
        cr.set_font_size(9.0);
        let color = match anim.path_status {
            PathStatus::Clear => (0.06, 0.72, 0.5),
            PathStatus::Altered => (0.94, 0.27, 0.27),
            PathStatus::Unknown => (0.96, 0.62, 0.04),
            PathStatus::Idle => (0.58, 0.64, 0.72),
        };
        cr.set_source_rgb(color.0, color.1, color.2);
        cr.move_to(inset + 3.0, h - 9.0);
        cr.show_text(&fitted_text(cr, &anim.path_reason, w - 2.0 * inset - 6.0))
            .ok();
    }
    cr.restore().ok();
}

pub fn scope(cr: &Context, w: f64, h: f64, jitter: &[f64]) {
    if jitter.len() < 2 {
        return;
    }

    let x_pad = 36.0;
    let y_pad_top = 12.0;
    let y_pad_bot = 18.0;
    let graph_w = (w - x_pad - 4.0).max(1.0);
    let graph_h = (h - y_pad_top - y_pad_bot).max(1.0);

    let step = graph_w / (jitter.len() - 1) as f64;

    cr.set_line_width(1.0);
    cr.set_source_rgba(0.2, 0.3, 0.4, 0.3);
    for i in 0..=2 {
        let y = y_pad_top + (graph_h / 2.0) * i as f64;
        cr.move_to(x_pad, y);
        cr.line_to(w - 4.0, y);
        let _ = cr.stroke();
    }

    cr.set_source_rgba(0.06, 0.72, 0.5, 0.18);
    cr.move_to(x_pad, h - y_pad_bot);
    for (i, v) in jitter.iter().enumerate() {
        let x = x_pad + i as f64 * step;
        let y = y_pad_top + (1.0 - v / 100.0) * graph_h;
        cr.line_to(x, y);
    }
    cr.line_to(w - 4.0, h - y_pad_bot);
    cr.close_path();
    let _ = cr.fill();

    cr.set_line_width(2.0);
    cr.set_source_rgb(0.06, 0.85, 0.6);
    for (i, v) in jitter.iter().enumerate() {
        let x = x_pad + i as f64 * step;
        let y = y_pad_top + (1.0 - v / 100.0) * graph_h;
        if i == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    let _ = cr.stroke();

    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(9.5);
    cr.set_source_rgba(0.5, 0.6, 0.7, 0.9);

    cr.move_to(4.0, y_pad_top + 4.0);
    let _ = cr.show_text("1ms");
    cr.move_to(4.0, y_pad_top + (graph_h / 2.0) + 4.0);
    let _ = cr.show_text("500");
    cr.move_to(10.0, y_pad_top + graph_h);
    let _ = cr.show_text("0");

    // 30 samples at 2 s pw-top cadence = 60 s window.
    cr.move_to(x_pad, h - 4.0);
    let _ = cr.show_text("-60s");
    cr.move_to(x_pad + graph_w / 2.0 - 12.0, h - 4.0);
    let _ = cr.show_text("-30s");
    cr.move_to(w - 26.0, h - 4.0);
    let _ = cr.show_text("Now");
}

/// Real stereo dBFS VU meter with independent Left/Right RMS bars, peak-hold
/// markers, and digital full-scale clip LEDs.
pub fn vu(cr: &Context, w: f64, h: f64, levels: &VuLevels) {
    let label_w = 11.0;
    let bars = 20usize;
    let gap = 1.8;
    let avail_w = (w - label_w - 2.0).max(20.0);
    let bw = ((avail_w - (bars as f64) * gap) / bars as f64).max(2.0);
    let bar_h = ((h - 5.0) / 2.0).clamp(4.0, 7.0);

    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size(8.0);

    let channels = [
        ("L", levels.left, levels.peak_l, levels.clip_l),
        ("R", levels.right, levels.peak_r, levels.clip_r),
    ];

    for (ch, (tag, rms, peak, clip)) in channels.iter().enumerate() {
        let y = 1.5 + ch as f64 * (bar_h + 2.5);

        cr.set_source_rgba(0.58, 0.64, 0.72, 0.9);
        cr.move_to(0.0, y + bar_h - 0.5);
        let _ = cr.show_text(tag);

        let active_bars = ((*rms * bars as f64).round() as usize).min(bars);
        let peak_idx = if *peak > 0.02 {
            Some(((*peak * (bars - 1) as f64).round() as usize).min(bars - 1))
        } else {
            None
        };

        for i in 0..bars {
            let frac = (i + 1) as f64 / bars as f64;
            let is_lit = i < active_bars;
            let is_peak = peak_idx == Some(i);
            let is_clip_led = i == bars - 1 && *clip;

            if is_clip_led {
                cr.set_source_rgb(0.98, 0.2, 0.2);
            } else if is_peak {
                if frac > 0.90 {
                    cr.set_source_rgb(0.98, 0.3, 0.3);
                } else if frac > 0.75 {
                    cr.set_source_rgb(0.99, 0.75, 0.15);
                } else {
                    cr.set_source_rgb(0.2, 0.95, 0.75);
                }
            } else if is_lit {
                if frac > 0.90 {
                    cr.set_source_rgb(0.94, 0.27, 0.27);
                } else if frac > 0.75 {
                    cr.set_source_rgb(0.96, 0.62, 0.04);
                } else {
                    cr.set_source_rgb(0.06, 0.72, 0.5);
                }
            } else {
                cr.set_source_rgba(0.15, 0.2, 0.25, 0.55);
            }

            let x = label_w + i as f64 * (bw + gap);
            cr.rectangle(x, y, bw, bar_h);
            let _ = cr.fill();
        }
    }
}

pub fn spectrum(cr: &Context, w: f64, h: f64, heights: &[f64]) {
    cr.set_source_rgba(0.03, 0.05, 0.07, 0.9);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let bands = heights.len();
    if bands == 0 {
        return;
    }
    let bw = (3.0f64).max((w - bands as f64 * 3.0) / bands as f64);

    for (i, &height) in heights.iter().enumerate() {
        let bh = height / 100.0 * (h - 14.0);
        let x = 4.0 + i as f64 * (bw + 2.5);
        let y = h - bh - 4.0;

        cr.set_source_rgba(0.06, 0.72, 0.5, 0.85);
        cr.rectangle(x, y, bw, bh);
        let _ = cr.fill();

        cr.set_source_rgba(0.13, 0.83, 0.93, 0.95);
        cr.rectangle(x, y, bw, bh.min(6.0));
        let _ = cr.fill();
    }
}
