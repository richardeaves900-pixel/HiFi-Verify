use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-configurable external tool shortcuts.
/// Empty strings trigger desktop-agnostic auto-detection on the host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct AppConfig {
    /// Music player command launched when clicking the Player/Source card.
    /// Empty disables auto-launch unless one of the common players is installed
    /// or the user configures their preferred player.
    pub player_cmd: String,
    /// Desktop audio settings mixer command.
    /// Empty auto-detects KDE, GNOME, pwvucontrol, or pavucontrol.
    pub audio_settings_cmd: String,
    /// DSP / EQ application command (empty auto-detects easyeffects / jamesdsp).
    pub dsp_cmd: String,
    /// PipeWire patchbay / graph viewer (empty auto-detects qpwgraph / helvum / carla).
    pub patchbay_cmd: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ShortcutKind {
    Player,
    AudioSettings,
    Dsp,
    Patchbay,
}

impl AppConfig {
    pub fn path() -> PathBuf {
        let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        dir.push("hifi-verify");
        dir.push("config.json");
        dir
    }

    pub fn load() -> Self {
        let path = Self::path();
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())
    }

    pub fn configured_command(&self, kind: ShortcutKind) -> &str {
        match kind {
            ShortcutKind::Player => self.player_cmd.trim(),
            ShortcutKind::AudioSettings => self.audio_settings_cmd.trim(),
            ShortcutKind::Dsp => self.dsp_cmd.trim(),
            ShortcutKind::Patchbay => self.patchbay_cmd.trim(),
        }
    }

    /// Resolve a shortcut into `(program, args)` on a background thread.
    /// Checks the user's configured command first, then desktop-agnostic fallbacks.
    pub fn resolve_command(&self, kind: ShortcutKind) -> Option<(String, Vec<String>)> {
        let configured = self.configured_command(kind);
        if !configured.is_empty() {
            return split_command_line(configured);
        }

        let candidates: &[&[&str]] = match kind {
            ShortcutKind::Player => &[
                &["strawberry"],
                &["elisa"],
                &["rhythmbox"],
                &["lollypop"],
                &["clementine"],
                &["feishin"],
                &["high-tide"],
                &["deadbeef"],
                &["audacious"],
            ],
            ShortcutKind::AudioSettings => &[
                &["kcmshell6", "kcm_pulseaudio"],
                &["gnome-control-center", "sound"],
                &["pwvucontrol"],
                &["pavucontrol"],
                &["pavucontrol-qt"],
            ],
            ShortcutKind::Dsp => &[&["easyeffects"], &["jamesdsp"]],
            ShortcutKind::Patchbay => &[&["qpwgraph"], &["helvum"], &["carla"]],
        };

        for candidate in candidates {
            if let Some(bin) = candidate.first() {
                if crate::host::command_exists_on_host(bin) {
                    return Some((
                        (*bin).to_string(),
                        candidate[1..].iter().map(|s| (*s).to_string()).collect(),
                    ));
                }
            }
        }
        None
    }

    /// Launch the configured or auto-detected command asynchronously (never blocks UI).
    pub fn launch_async(&self, kind: ShortcutKind) {
        let cfg = self.clone();
        std::thread::spawn(move || {
            if let Some((bin, args)) = cfg.resolve_command(kind) {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                crate::host::host_popen(&bin, &refs);
            }
        });
    }
}

/// Minimal shell-like argv splitter supporting single and double quotes.
pub fn split_command_line(input: &str) -> Option<(String, Vec<String>)> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for ch in input.trim().chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }

    let mut iter = parts.into_iter();
    let bin = iter.next()?;
    Some((bin, iter.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_command_lines() {
        let (bin, args) =
            split_command_line("flatpak run \"io.github.nokse22.high-tide\" --arg 'my val'")
                .unwrap();
        assert_eq!(bin, "flatpak");
        assert_eq!(
            args,
            vec!["run", "io.github.nokse22.high-tide", "--arg", "my val"]
        );
    }

    #[test]
    fn empty_command_line_returns_none() {
        assert!(split_command_line("   ").is_none());
    }

    #[test]
    fn config_roundtrip_serde() {
        let cfg = AppConfig {
            player_cmd: "strawberry".into(),
            audio_settings_cmd: "pavucontrol".into(),
            dsp_cmd: "easyeffects".into(),
            patchbay_cmd: "helvum".into(),
        };
        let raw = serde_json::to_string(&cfg).unwrap();
        let parsed: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, cfg);
    }
}
