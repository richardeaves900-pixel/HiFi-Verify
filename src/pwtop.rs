use crate::host::host_cmd;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct NodeRow {
    pub wait_us: Option<f64>,
    pub status: char,
    pub id: String,
    pub quant: String,
    pub rate: String,
    pub wait: String,
    pub busy: String,
    pub wq: f32,
    pub bq: f32,
    pub err: u64,
    pub fmt: String,
    /// Negotiated stream format sample rate from the `pw-top` FORMAT column
    /// (e.g., `S24LE 2 96000` -> `Some(96000)`), distinct from the graph RATE column.
    pub fmt_rate: Option<u32>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PwTopSnapshot {
    pub rows: Vec<NodeRow>,
    pub total_err: u64,
}

fn parse_time_us(s: &str) -> Option<f64> {
    if s == "?" || s == "---" || s.is_empty() {
        return None;
    }
    if let Some(us) = s.strip_suffix("us") {
        return us.parse::<f64>().ok();
    }
    if let Some(ms) = s.strip_suffix("ms") {
        return ms.parse::<f64>().map(|v| v * 1000.0).ok();
    }
    if let Some(sec) = s.strip_suffix('s') {
        return sec.parse::<f64>().map(|v| v * 1_000_000.0).ok();
    }
    s.parse::<f64>().ok()
}

/// PipeWire `pw-top` prints `W/Q` and `B/Q` as dimensionless ratios (`0.04` = 4%,
/// `1.05` = 105% overrun). Values explicitly suffixed with `%` are kept as-is.
fn parse_pct(s: &str) -> f32 {
    if let Some(pct) = s.strip_suffix('%') {
        pct.parse::<f32>().unwrap_or(0.0)
    } else {
        s.parse::<f32>().map(|v| v * 100.0).unwrap_or(0.0)
    }
}

fn looks_like_audio_format(token: &str) -> bool {
    let t = token.to_ascii_uppercase();
    (t.starts_with('S') || t.starts_with('U') || t.starts_with('F') || t.starts_with("DSD"))
        && t.chars().any(|c| c.is_ascii_digit())
}

pub fn parse_pw_top_output(text: &str) -> Option<PwTopSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let header_idx = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("S ") && l.contains("QUANT"))?;
    let start = header_idx + 1;

    let mut rows = Vec::with_capacity(16);
    let mut total_err = 0u64;

    for line in &lines[start..] {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 10 {
            continue;
        }
        let status = p[0].chars().next().unwrap_or('?');
        let err: u64 = p[8].parse().unwrap_or(0);
        total_err += err;

        let (fmt, fmt_rate, name) = if p.len() >= 13
            && looks_like_audio_format(p[9])
            && p[10].parse::<u32>().is_ok()
            && p[11].parse::<u32>().is_ok()
        {
            let rate = p[11]
                .parse::<u32>()
                .ok()
                .filter(|r| (8000..=768_000).contains(r));
            (format!("{} {}c", p[9], p[10]), rate, p[12..].join(" "))
        } else if p.len() >= 12 && looks_like_audio_format(p[9]) && p[10].parse::<u32>().is_ok() {
            (format!("{} {}c", p[9], p[10]), None, p[11..].join(" "))
        } else {
            ("-".to_string(), None, p[9..].join(" "))
        };

        rows.push(NodeRow {
            status,
            id: p[1].to_string(),
            quant: p[2].to_string(),
            rate: p[3].to_string(),
            wait: p[4].to_string(),
            busy: p[5].to_string(),
            wq: parse_pct(p[6]),
            bq: parse_pct(p[7]),
            err,
            fmt,
            fmt_rate,
            name,
            wait_us: parse_time_us(p[4]),
        });
    }

    Some(PwTopSnapshot { rows, total_err })
}

/// Execute `pw-top -b -n 2` and parse the latest frame.
/// Returns `None` if `pw-top` fails or times out so callers do not treat a
/// transient failure as `total_err == 0`.
pub fn parse_pw_top() -> Option<PwTopSnapshot> {
    let out = host_cmd(&["pw-top", "-b", "-n", "2"], Duration::from_secs(4)).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_pw_top_output(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pw_top_format_rate_for_follower_streams_and_overruns() {
        let sample = "\
S   ID  QUANT   RATE    WAIT    BUSY   W/Q   B/Q  ERR FORMAT           NAME
S   29      0      0    ---     ---   ---   ---    0                  Dummy-Driver
R   44    512  96000  28.4us  41.2us  0.01  0.02    0    S32LE 2 96000 alsa_output.usb-Fiio
R   12      0      0  14.0us   9.5us  1.05  0.00    2    S24LE 2 96000 python3.13
";
        let snap = parse_pw_top_output(sample).unwrap();
        assert_eq!(snap.rows.len(), 3);
        assert_eq!(snap.total_err, 2);

        let stream = &snap.rows[2];
        assert_eq!(stream.id, "12");
        assert_eq!(stream.rate, "0");
        assert_eq!(stream.fmt, "S24LE 2c");
        assert_eq!(stream.fmt_rate, Some(96000));
        assert_eq!(stream.name, "python3.13");
        assert!((stream.wq - 105.0).abs() < 0.01);
        assert_eq!(stream.wait_us, Some(14.0));
    }

    #[test]
    fn parse_pct_handles_ratios_and_explicit_percentages() {
        assert!((parse_pct("0.04") - 4.0).abs() < 0.01);
        assert!((parse_pct("1.00") - 100.0).abs() < 0.01);
        assert!((parse_pct("1.25") - 125.0).abs() < 0.01);
        assert!((parse_pct("12.5%") - 12.5).abs() < 0.01);
        assert_eq!(parse_pct("---"), 0.0);
    }
}
