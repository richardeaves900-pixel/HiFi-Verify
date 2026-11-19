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

/// Compact kHz badge for tight toolbar pills (e.g., 44100 -> "44.1kHz", 96000 -> "96kHz").
pub fn format_rate_compact(rate_hz: u32) -> String {
    let khz = rate_hz as f64 / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{}kHz", khz.round() as u32)
    } else {
        format!("{:.1}kHz", khz)
    }
}

/// Hi-Fi friendly `[Bit Depth]/[Sample Rate]` display (e.g. `24-bit/192kHz`, `32-bit/44.1kHz`).
pub fn format_hifi_rate(bits: Option<u8>, rate_hz: u32) -> String {
    let sr = format_rate_compact(rate_hz);
    match bits {
        Some(b) if b > 0 => format!("{b}-bit/{sr}"),
        _ => format_rate_khz(rate_hz),
    }
}

/// Uncompressed PCM data rate (e.g. `1411 kbps`, `4608 kbps`, `12.3 Mbps`).
pub fn format_pcm_bitrate(rate_hz: u32, bits: u8, channels: u32) -> String {
    let bps = rate_hz as u64 * bits as u64 * channels as u64;
    let kbps = bps / 1000;
    if kbps >= 10_000 {
        format!("{:.1} Mbps", bps as f64 / 1_000_000.0)
    } else {
        format!("{kbps} kbps")
    }
}

/// Hi-Fi tier and uncompressed PCM bitrate subtitle shared by GUI and `--status`.
/// Example: `"Hi-Res PCM • 12.3 Mbps"`, `"CD Quality PCM • 1411 kbps"`, `"Standard PCM"`.
pub fn format_hifi_subtitle(bits: Option<u8>, rate_hz: u32, channels: Option<u32>) -> String {
    let tier = if rate_hz >= 88_200 {
        "Hi-Res PCM"
    } else if rate_hz == 44_100 && bits == Some(16) {
        "CD Quality PCM"
    } else {
        "Standard PCM"
    };
    if let (Some(b), Some(ch)) = (bits, channels) {
        format!("{tier} • {}", format_pcm_bitrate(rate_hz, b, ch))
    } else {
        tier.to_string()
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

    #[test]
    fn formats_hifi_bit_depth_and_sample_rate() {
        assert_eq!(format_hifi_rate(Some(32), 192_000), "32-bit/192kHz");
        assert_eq!(format_hifi_rate(Some(24), 44_100), "24-bit/44.1kHz");
        assert_eq!(format_hifi_rate(Some(16), 48_000), "16-bit/48kHz");
        assert_eq!(format_hifi_rate(None, 192_000), "192.0 kHz");
    }

    #[test]
    fn formats_uncompressed_pcm_bitrates() {
        assert_eq!(format_pcm_bitrate(44_100, 16, 2), "1411 kbps");
        assert_eq!(format_pcm_bitrate(96_000, 24, 2), "4608 kbps");
        assert_eq!(format_pcm_bitrate(192_000, 32, 2), "12.3 Mbps");
    }
}
