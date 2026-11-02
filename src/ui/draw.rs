use cairo::Context;

pub struct Anim {
    pub jitter: Vec<f64>,
    pub dsp_load: f64,
    pub playing: bool,
    pub vu_level: f64,
    pub matrix_src_app: String,
    pub matrix_src_codec: String,
    pub matrix_dsp: String,
    pub matrix_hw_fmt: String,
    pub matrix_rate: f64,
    pub is_processing: bool,
    pub alsa_bitperfect: bool,
    pub alsa_reason: String,
    #[allow(dead_code)]
    pub heights: Vec<f64>,
}

impl Anim {
    pub fn new() -> Self {
        Self {
            jitter: vec![0.0; 50],
            dsp_load: 1.8,
            playing: false,
            vu_level: 0.0,
            matrix_src_app: "Detecting...".into(),
            matrix_src_codec: "---".into(),
            matrix_dsp: "---".into(),
            matrix_hw_fmt: "---".into(),
            matrix_rate: 0.0,
            is_processing: false,
            alsa_bitperfect: false,
            alsa_reason: String::new(),
            heights: vec![0.0; 32],
        }
    }

    pub fn update(&mut self, t: f64) {
        if self.playing {
            let base = (t * 8.0).sin().abs() * 0.6 + 0.3;
            let noise = ((t * 45.0).sin() * 0.15) + ((t * 13.0).cos() * 0.1);
            let target = (base + noise).clamp(0.05, 0.95);
            self.vu_level += (target - self.vu_level) * 0.4;
        } else {
            self.vu_level -= 0.05;
            if self.vu_level < 0.0 {
                self.vu_level = 0.0;
            }
        }
    }
}

pub fn gauge(cr: &Context, w: f64, h: f64, load: f64) {
    let (cx, cy) = (w / 2.0, h - 6.0);
    let r = (w / 2.0 - 12.0).min(h - 10.0);
    cr.set_line_width(6.0);
    cr.set_source_rgba(0.1, 0.2, 0.18, 0.8);
    cr.arc(cx, cy, r, std::f64::consts::PI, 2.0 * std::f64::consts::PI);
    let _ = cr.stroke();

    let frac = (load / 25.0).clamp(0.05, 1.0);
    cr.set_source_rgb(0.06, 0.72, 0.5);
    cr.arc(
        cx,
        cy,
        r,
        std::f64::consts::PI,
        std::f64::consts::PI + frac * std::f64::consts::PI,
    );
    let _ = cr.stroke();

    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size(16.0);
    let txt = format!("{:.1}%", load);
    if let Ok(ext) = cr.text_extents(&txt) {
        cr.move_to(cx - ext.width() / 2.0, cy - 4.0);
        let _ = cr.show_text(&txt);
    }
}

pub fn chain_matrix(cr: &Context, w: f64, h: f64, anim: &Anim) {
    cr.set_source_rgba(0.03, 0.05, 0.07, 0.9);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size(12.0);

    let box_w = w - 40.0;
    let box_h = 50.0;
    let x = 20.0;

    let trunc = |s: &str, max: usize| -> String {
        if s.len() > max {
            format!("{}...", &s[0..max - 3])
        } else {
            s.to_string()
        }
    };

    let src_text = format!(
        "{} | {}",
        trunc(&anim.matrix_src_app, 15),
        trunc(&anim.matrix_src_codec, 20)
    );
    let dsp_text = if anim.matrix_dsp.is_empty() || anim.matrix_dsp == "---" {
        "NONE (Bit-Perfect)".to_string()
    } else {
        trunc(&anim.matrix_dsp, 38)
    };
    let hw_text = format!("{} @ {:.1}kHz", anim.matrix_hw_fmt, anim.matrix_rate);

    let steps = [
        ("SOURCE", &src_text, 0.6, 0.4, 1.0),
        ("DSP / PROCESSING", &dsp_text, 1.0, 0.8, 0.4),
        ("DAC BUS", &hw_text, 0.4, 0.8, 0.6),
    ];

    let gap = (h - (3.0 * box_h)) / 4.0;
    let mut last_y = 0.0;

    for (i, (title, val, r, g, b)) in steps.iter().enumerate() {
        let y = gap + (i as f64 * (box_h + gap));

        cr.set_source_rgba(*r, *g, *b, 0.1);
        cr.rectangle(x, y, box_w, box_h);
        let _ = cr.fill();

        cr.set_source_rgba(*r, *g, *b, 0.8);
        cr.rectangle(x, y, box_w, box_h);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        cr.set_source_rgba(*r, *g, *b, 0.9);
        cr.move_to(x + 10.0, y + 18.0);
        let _ = cr.show_text(title);

        cr.set_source_rgba(0.9, 0.9, 0.9, 1.0);
        cr.move_to(x + 10.0, y + 38.0);
        let _ = cr.show_text(val);

        if i < 2 {
            let arrow_x = w / 2.0;
            let arrow_y_start = y + box_h;
            let arrow_y_end = y + box_h + gap;

            cr.set_source_rgba(0.5, 0.6, 0.7, 0.5);
            cr.set_line_width(2.0);
            cr.move_to(arrow_x, arrow_y_start);
            cr.line_to(arrow_x, arrow_y_end);
            let _ = cr.stroke();

            cr.move_to(arrow_x - 5.0, arrow_y_end - 8.0);
            cr.line_to(arrow_x, arrow_y_end);
            cr.line_to(arrow_x + 5.0, arrow_y_end - 8.0);
            let _ = cr.stroke();
        }
        last_y = y;
    }

    cr.set_source_rgba(0.5, 0.6, 0.7, 0.8);
    cr.set_font_size(10.0);
    cr.move_to(x + 10.0, last_y + box_h + gap + 10.0);
    let _ = cr.show_text(&format!("Hardware Path: {}", hw_text));

    // ALSA Bit-Perfect Status
    let (color, icon) = if anim.alsa_bitperfect {
        ((0.06, 0.72, 0.5), "✓")
    } else {
        ((0.96, 0.62, 0.04), "⚠")
    };
    cr.set_source_rgba(color.0, color.1, color.2, 1.0);
    cr.move_to(x + 10.0, last_y + box_h + gap + 28.0);
    let _ = cr.show_text(&format!("{} ALSA: {}", icon, anim.alsa_reason));
}

pub fn scope(cr: &Context, w: f64, h: f64, jitter: &[f64]) {
    if jitter.len() < 2 {
        return;
    }

    let x_pad = 36.0;
    let y_pad_top = 12.0;
    let y_pad_bot = 18.0;
    let graph_w = w - x_pad - 4.0;
    let graph_h = h - y_pad_top - y_pad_bot;

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
    cr.set_font_size(10.0);
    cr.set_source_rgba(0.5, 0.6, 0.7, 0.9);

    let _ = cr.move_to(4.0, y_pad_top + 4.0);
    let _ = cr.show_text("1ms");
    let _ = cr.move_to(4.0, y_pad_top + (graph_h / 2.0) + 4.0);
    let _ = cr.show_text("500");
    let _ = cr.move_to(10.0, y_pad_top + graph_h);
    let _ = cr.show_text("0");

    let _ = cr.move_to(x_pad, h - 4.0);
    let _ = cr.show_text("-4s");
    let _ = cr.move_to(x_pad + graph_w / 2.0 - 10.0, h - 4.0);
    let _ = cr.show_text("-2s");
    let _ = cr.move_to(w - 30.0, h - 4.0);
    let _ = cr.show_text("Now");
}

pub fn vu(cr: &Context, w: f64, h: f64, level: f64) {
    let _ = h;
    let bars = 18;
    let bw = (2.0f64).max((w - bars as f64 * 2.0) / bars as f64);
    let active = (bars as f64 * level) as usize;

    for ch in 0..2 {
        let y = 3.0 + ch as f64 * 10.0;
        let ch_active = if ch == 1 && active > 1 {
            active - 1
        } else {
            active
        };

        for i in 0..bars {
            if i < ch_active {
                if (i as f64) > bars as f64 * 0.8 {
                    cr.set_source_rgb(0.96, 0.62, 0.04);
                } else {
                    cr.set_source_rgb(0.06, 0.72, 0.5);
                }
            } else {
                cr.set_source_rgba(0.15, 0.2, 0.25, 0.6);
            }
            cr.rectangle(i as f64 * (bw + 2.0), y, bw, 6.0);
            let _ = cr.fill();
        }
    }
}

pub fn spectrum(cr: &Context, w: f64, h: f64, heights: &[f64]) {
    cr.set_source_rgba(0.03, 0.05, 0.07, 0.9);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let bands = heights.len();
    let bw = (3.0f64).max((w - bands as f64 * 3.0) / bands as f64);

    for i in 0..bands {
        let bh = heights[i] / 100.0 * (h - 14.0);
        let x = 4.0 + i as f64 * (bw + 2.5);
        let y = h - bh - 4.0;

        // Main bar (emerald)
        cr.set_source_rgba(0.06, 0.72, 0.5, 0.85);
        cr.rectangle(x, y, bw, bh);
        let _ = cr.fill();

        // Peak cap (cyan)
        cr.set_source_rgba(0.13, 0.83, 0.93, 0.95);
        cr.rectangle(x, y, bw, bh.min(6.0));
        let _ = cr.fill();
    }
}
