use std::sync::mpsc::Sender;
use ksni::menu::{MenuItem, StandardItem};

pub struct HiFiTray {
    pub tx: Sender<()>,
    pub status_label: String,
    pub is_bitperfect: bool,
    pub is_playing: bool,
    pub app_name: String,
    pub stream_info: String,
    pub hw_info: String,
    pub dsp_info: String,
}

impl HiFiTray {
    pub fn new(tx: Sender<()>) -> Self {
        Self {
            tx,
            status_label: "STANDBY".into(),
            is_bitperfect: false,
            is_playing: false,
            app_name: "Idle".into(),
            stream_info: "---".into(),
            hw_info: "---".into(),
            dsp_info: "None".into(),
        }
    }
}

impl ksni::Tray for HiFiTray {
    fn id(&self) -> String {
        "hifi-verify".into()
    }

    fn title(&self) -> String {
        format!("HiFi Verify [{}]", self.status_label)
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    // Hover Tooltip in KDE Plasma
    fn tool_tip(&self) -> ksni::ToolTip {
        let title = if !self.is_playing {
            "HiFi Verify — ● STANDBY".to_string()
        } else if self.is_bitperfect {
            "HiFi Verify — ● BIT-PERFECT".to_string()
        } else {
            format!("HiFi Verify — ● {}", self.status_label)
        };

        let description = if !self.is_playing {
            format!("No active audio stream\nDAC Bus: {}\nDSP: {}", self.hw_info, self.dsp_info)
        } else {
            format!(
                "Source: {} ({})\nDAC Bus: {}\nDSP Chain: {}",
                self.app_name, self.stream_info, self.hw_info, self.dsp_info
            )
        };

        ksni::ToolTip {
            title,
            description,
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    // Procedural 32x32 ARGB32 badge: Emerald (Bit-Perfect), Red (Compromised), Slate (Standby)
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let width = 32;
        let height = 32;
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        let (cx, cy) = (15.5_f32, 15.5_f32);

        let (ring_r, ring_g, ring_b, bar_r, bar_g, bar_b) = if !self.is_playing {
            // Standby: Muted Slate
            (100, 116, 139, 148, 163, 184)
        } else if self.is_bitperfect {
            // Bit-Perfect: Emerald Green
            (15, 184, 128, 15, 217, 153)
        } else {
            // Resampled / DSP Active: Ruby Red
            (239, 68, 68, 251, 146, 60)
        };

        for y in 0..height {
            for x in 0..width {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();

                let (a, r, g, b) = if dist <= 13.5 && dist >= 10.5 {
                    (255, ring_r, ring_g, ring_b)
                } else if dist < 10.5 {
                    let bar = (x == 11 && (12..=19).contains(&y))
                        || (x == 15 && (9..=22).contains(&y))
                        || (x == 19 && (13..=18).contains(&y));
                    if bar {
                        (255, bar_r, bar_g, bar_b)
                    } else {
                        (240, 12, 20, 28)
                    }
                } else {
                    (0, 0, 0, 0)
                };

                data.extend_from_slice(&[a, r, g, b]);
            }
        }

        vec![ksni::Icon {
            width,
            height,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(());
    }

    fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
        // delta is usually +1 (up) or -1 (down)
        if delta > 0 {
            crate::volume::change_volume(0.05);
        } else if delta < 0 {
            crate::volume::change_volume(-0.05);
        }
    }

    // Right-Click Context Menu with live playback telemetry
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let header = if !self.is_playing {
            "● STATUS: STANDBY (Idle)".to_string()
        } else if self.is_bitperfect {
            "✓ BIT-PERFECT: YES (Direct Hardware)".to_string()
        } else {
            format!("✗ BIT-PERFECT: NO ({})", self.status_label)
        };

        // Helper to create un-clickable but white text items
        let info_item = |label: String| StandardItem {
            label,
            enabled: true,
            activate: Box::new(|_: &mut Self| {}),
            ..Default::default()
        };

        vec![
            info_item(header).into(),
            MenuItem::Separator,
            info_item(format!("Source App:  {}", self.app_name)).into(),
            info_item(format!("Stream Format: {}", self.stream_info)).into(),
            info_item(format!("DAC Hardware: {}", self.hw_info)).into(),
            info_item(format!("DSP / Filters: {}", self.dsp_info)).into(),
            MenuItem::Separator,
            StandardItem {
                label: "⏮ Previous".into(),
                activate: Box::new(|_| {
                    crate::telemetry::send_mpris_command("Previous");
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "⏯ Play / Pause".into(),
                activate: Box::new(|_| {
                    crate::telemetry::send_mpris_command("PlayPause");
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "⏭ Next".into(),
                activate: Box::new(|_| {
                    crate::telemetry::send_mpris_command("Next");
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "🔊 Volume Up (+5%)".into(),
                activate: Box::new(|_| {
                    crate::volume::change_volume(0.05);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "🔉 Volume Down (-5%)".into(),
                activate: Box::new(|_| {
                    crate::volume::change_volume(-0.05);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "🔇 Mute / Unmute".into(),
                activate: Box::new(|_| {
                    let _ = crate::host::host_cmd(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"], std::time::Duration::from_millis(200));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Show / Hide HiFi Verify".into(),
                icon_name: "window-restore".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(());
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
