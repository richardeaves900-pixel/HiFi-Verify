use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct HwParams {
    pub path: String,
    pub format: String,
    pub rate: u32,
    pub channels: Option<u32>,
    pub period_size: Option<u32>,
    pub buffer_size: Option<u32>,
    pub bits: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sink {
    pub id: String,
    pub description: String,
}

/// Decoded *PipeWire* playback PCM, not a claim about the source file's codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackPcm {
    pub format: String,
    pub rate: u32,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathStatus {
    #[default]
    Idle,
    Clear,
    Altered,
    Unknown,
}

impl PathStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Clear => "clear",
            Self::Altered => "altered",
            Self::Unknown => "unknown",
        }
    }

    pub const fn widget_state(self) -> &'static str {
        match self {
            Self::Idle => "gray",
            Self::Clear => "green",
            Self::Altered => "red",
            Self::Unknown => "amber",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Idle => "STANDBY",
            Self::Clear => "CLEAR",
            Self::Altered => "ALTERED",
            Self::Unknown => "UNVERIFIED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcmFormatStatus {
    Match,
    RateMismatch,
    ChannelMismatch,
    PrecisionReduction,
    #[default]
    Unknown,
}

impl PcmFormatStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::RateMismatch => "rate_mismatch",
            Self::ChannelMismatch => "channel_mismatch",
            Self::PrecisionReduction => "precision_reduction",
            Self::Unknown => "unknown",
        }
    }
}

/// Extract nominal sample bit depth from either ALSA (`S32_LE`, `S24_3LE`, `FLOAT_LE`)
/// or PipeWire (`S32LE`, `S24LE`, `F32LE`, `S16LE`) format identifiers.
pub fn pcm_format_bits(fmt: &str) -> Option<u8> {
    let upper = fmt.trim().to_ascii_uppercase();
    match upper.as_str() {
        "S8" | "U8" => return Some(8),
        "FLOAT_LE" | "FLOAT_BE" | "F32LE" | "F32BE" | "F32P" => return Some(32),
        "FLOAT64_LE" | "FLOAT64_BE" | "F64LE" | "F64BE" | "F64P" => return Some(64),
        _ => {}
    }
    let rest = upper
        .strip_prefix('S')
        .or_else(|| upper.strip_prefix('U'))
        .or_else(|| upper.strip_prefix('F'))?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    match digits.parse::<u8>().ok()? {
        b @ (8 | 16 | 24 | 32 | 64) => Some(b),
        _ => None,
    }
}

/// Format a sample rate in Hz cleanly as kHz (e.g., 44100 -> "44.1 kHz", 96000 -> "96.0 kHz").
pub fn format_rate_khz(rate_hz: u32) -> String {
    format!("{:.1} kHz", rate_hz as f64 / 1000.0)
}

/// Compact kHz badge for tight toolbar pills (e.g., 44100 -> "44.1k", 96000 -> "96k").
pub fn format_rate_compact(rate_hz: u32) -> String {
    let khz = rate_hz as f64 / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{}kHz", khz.round() as u32)
    } else {
        format!("{:.1}kHz", khz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_alsa_and_pipewire_format_bits() {
        assert_eq!(pcm_format_bits("S16_LE"), Some(16));
        assert_eq!(pcm_format_bits("S24_3LE"), Some(24));
        assert_eq!(pcm_format_bits("S24LE"), Some(24));
        assert_eq!(pcm_format_bits("S32_LE"), Some(32));
        assert_eq!(pcm_format_bits("S32LE"), Some(32));
        assert_eq!(pcm_format_bits("FLOAT_LE"), Some(32));
        assert_eq!(pcm_format_bits("F32LE"), Some(32));
        assert_eq!(pcm_format_bits("FLOAT64_LE"), Some(64));
        assert_eq!(pcm_format_bits("F64LE"), Some(64));
        assert_eq!(pcm_format_bits("UNKNOWN"), None);
    }

    #[test]
    fn formats_compact_rates() {
        assert_eq!(format_rate_compact(44100), "44.1kHz");
        assert_eq!(format_rate_compact(48000), "48kHz");
        assert_eq!(format_rate_compact(96000), "96kHz");
        assert_eq!(format_rate_compact(88200), "88.2kHz");
        assert_eq!(format_rate_compact(192000), "192kHz");
    }
}
