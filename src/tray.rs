use crate::model::{format_rate_compact, PathStatus};
use ksni::menu::{MenuItem, StandardItem};
use std::sync::mpsc::Sender;

/// Actions sent from the system tray thread back to the GTK main loop.
#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    ToggleWindow,
    Quit,
}

pub struct HiFiTray {
    pub tx: Sender<TrayAction>,
    pub status: PathStatus,
    pub status_label: String,
    pub dac_rate_hz: Option<u32>,
    pub dac_format: String,
    pub app_name: String,
    pub track_name: String,
    pub stream_info: String,
    pub hw_info: String,
    pub dsp_info: String,
    pub pipewire_info: String,
    pub xrun_info: String,
}

impl HiFiTray {
    pub fn new(tx: Sender<TrayAction>) -> Self {
        Self {
            tx,
            status: PathStatus::Idle,
            status_label: "STANDBY".into(),
            dac_rate_hz: None,
            dac_format: String::new(),
            app_name: "Idle".into(),
            track_name: String::new(),
            stream_info: "---".into(),
            hw_info: "No open playback PCM".into(),
            dsp_info: "None detected".into(),
            pipewire_info: "Unknown".into(),
            xrun_info: "0 session (0 total)".into(),
        }
    }

    fn at_a_glance_summary(&self) -> String {
        if self.status == PathStatus::Idle {
            return "Standby".into();
        }
        let mut parts = Vec::new();
        if let Some(rate) = self.dac_rate_hz {
            parts.push(format_rate_compact(rate));
        }
        if !self.dac_format.is_empty() && self.dac_format != "---" {
            parts.push(self.dac_format.clone());
        }
        let tag = self.status.short_label();
        if parts.is_empty() {
            tag.to_string()
        } else {
            format!("{} [{}]", parts.join(" "), tag)
        }
    }
}

/// Render a crisp procedural ARGB32 status icon at arbitrary square size.
///
/// Visual language:
/// - Outer ring color: Slate (Idle), Emerald (Clear), Amber (Unverified), Red (Altered)
/// - Center glyph: Dot (Idle), Checkmark (Clear), `?` (Unverified), `!` (Altered)
/// - Top-right cyan Hi-Res pip when Clear and DAC sample rate >= 88.2 kHz
fn render_icon(size: i32, status: PathStatus, is_hires: bool) -> ksni::Icon {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let s = size as f32;
    let c = (s - 1.0) * 0.5;
    let outer_r = s * 0.44;
    let inner_r = s * 0.34;

    let (ring_r, ring_g, ring_b, fg_r, fg_g, fg_b) = match status {
        PathStatus::Idle => (100, 116, 139, 148, 163, 184),
        PathStatus::Clear => (16, 185, 129, 52, 211, 153),
        PathStatus::Unknown => (245, 158, 11, 252, 211, 77),
        PathStatus::Altered => (239, 68, 68, 252, 165, 165),
    };

    for y in 0..size {
        for x in 0..size {
            let fx = x as f32;
            let fy = y as f32;
            let dx = fx - c;
            let dy = fy - c;
            let dist = (dx * dx + dy * dy).sqrt();

            // Normalized coordinates in [-1.0, 1.0] inside the icon.
            let nx = dx / (s * 0.5);
            let ny = dy / (s * 0.5);

            // Top-right cyan micro-badge when Clear & Hi-Res (>= 88.2 kHz).
            let pip_dx = nx - 0.68;
            let pip_dy = ny + 0.68;
            let pip_dist = (pip_dx * pip_dx + pip_dy * pip_dy).sqrt();
            if status == PathStatus::Clear && is_hires && pip_dist <= 0.22 {
                data.extend_from_slice(&[255, 34, 211, 238]);
                continue;
            }

            let (a, r, g, b) = if (inner_r..=outer_r).contains(&dist) {
                (255, ring_r, ring_g, ring_b)
            } else if dist < inner_r {
                let glyph = match status {
                    PathStatus::Idle => (nx * nx + ny * ny).sqrt() <= 0.16,
                    PathStatus::Clear => {
                        // Crisp checkmark composed of two line segments:
                        // (-0.32, 0.02) -> (-0.08, 0.26) -> (0.34, -0.24)
                        point_seg_dist(nx, ny, -0.32, 0.02, -0.08, 0.26) <= 0.095
                            || point_seg_dist(nx, ny, -0.08, 0.26, 0.34, -0.24) <= 0.095
                    }
                    PathStatus::Altered => {
                        // Exclamation mark `!`
                        (nx.abs() <= 0.095 && (-0.36..=0.10).contains(&ny))
                            || (nx * nx + (ny - 0.29) * (ny - 0.29)).sqrt() <= 0.105
                    }
                    PathStatus::Unknown => {
                        // Question mark `?`
                        let arc_d = ((nx * nx + (ny + 0.16) * (ny + 0.16)).sqrt() - 0.18).abs();
                        let top_arc = arc_d <= 0.075 && (ny <= -0.14 || nx >= 0.0);
                        let stem = nx.abs() <= 0.075 && (0.0..=0.12).contains(&ny);
                        let dot = (nx * nx + (ny - 0.29) * (ny - 0.29)).sqrt() <= 0.095;
                        top_arc || stem || dot
                    }
                };
                if glyph {
                    (255, fg_r, fg_g, fg_b)
                } else {
                    (242, 10, 17, 24)
                }
            } else {
                (0, 0, 0, 0)
            };

            data.extend_from_slice(&[a, r, g, b]);
        }
    }

    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

fn point_seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let len_sq = abx * abx + aby * aby;
    if len_sq <= 1e-6 {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = (((px - ax) * abx + (py - ay) * aby) / len_sq).clamp(0.0, 1.0);
    let cx = ax + t * abx;
    let cy = ay + t * aby;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

impl ksni::Tray for HiFiTray {
    fn id(&self) -> String {
        "hifi-verify".into()
    }

    fn title(&self) -> String {
        format!("HiFi Verify · {}", self.at_a_glance_summary())
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let title = format!("HiFi Verify — {}", self.at_a_glance_summary());
        let mut lines = vec![
            format!("Status: {}", self.status_label),
            format!("Player: {}", self.app_name),
        ];
        if !self.track_name.is_empty() && self.track_name != self.app_name {
            lines.push(format!("Track: {}", self.track_name));
        }
        lines.push(format!("Source: {}", self.stream_info));
        lines.push(format!("PipeWire: {}", self.pipewire_info));
        lines.push(format!("DAC: {}", self.hw_info));
        lines.push(format!("DSP: {}", self.dsp_info));
        lines.push(format!("XRUNs: {}", self.xrun_info));

        ksni::ToolTip {
            title,
            description: lines.join("\n"),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let is_hires = self.dac_rate_hz.is_some_and(|r| r >= 88_200);
        vec![
            render_icon(32, self.status, is_hires),
            render_icon(64, self.status, is_hires),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayAction::ToggleWindow);
    }

    fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
        let step = if delta > 0 {
            0.05
        } else if delta < 0 {
            -0.05
        } else {
            return;
        };
        std::thread::spawn(move || crate::volume::change_volume(step));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let header = format!("● {}", self.at_a_glance_summary());

        let info_item = |label: String| StandardItem {
            label,
            enabled: false,
            ..Default::default()
        };

        vec![
            info_item(header).into(),
            info_item(self.status_label.clone()).into(),
            MenuItem::Separator,
            info_item(format!("Player: {}", self.app_name)).into(),
            info_item(format!("Source: {}", self.stream_info)).into(),
            info_item(format!("PipeWire: {}", self.pipewire_info)).into(),
            info_item(format!("DAC: {}", self.hw_info)).into(),
            info_item(format!("DSP: {}", self.dsp_info)).into(),
            info_item(format!("XRUNs: {}", self.xrun_info)).into(),
            MenuItem::Separator,
            StandardItem {
                label: "⏮ Previous".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| crate::telemetry::send_mpris_command("Previous"));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "⏯ Play / Pause".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| crate::telemetry::send_mpris_command("PlayPause"));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "⏭ Next".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| crate::telemetry::send_mpris_command("Next"));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "🔊 Volume Up (+5%)".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| crate::volume::change_volume(0.05));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "🔉 Volume Down (-5%)".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| crate::volume::change_volume(-0.05));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "🔇 Mute / Unmute".into(),
                activate: Box::new(|_| {
                    std::thread::spawn(|| {
                        let _ = crate::host::host_cmd(
                            &["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
                            std::time::Duration::from_millis(500),
                        );
                    });
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Show / Hide HiFi Verify".into(),
                icon_name: "window-restore".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayAction::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayAction::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
