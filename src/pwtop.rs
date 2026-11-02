use crate::host::host_cmd;
use std::time::Duration;

#[derive(Clone)]
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
    pub name: String,
}

fn parse_time_us(s: &str) -> Option<f64> {
    if s == "?" || s.is_empty() {
        return None;
    }
    if let Some(us) = s.strip_suffix("us") {
        return us.parse::<f64>().ok();
    }
    if let Some(ms) = s.strip_suffix("ms") {
        return ms.parse::<f64>().map(|v| v * 1000.0).ok();
    }
    if let Some(sec) = s.strip_suffix("s") {
        return sec.parse::<f64>().map(|v| v * 1000000.0).ok();
    }
    s.parse::<f64>().ok()
}

pub fn parse_pw_top() -> (Vec<NodeRow>, Option<u32>, Option<u32>, u64) {
    let Ok(out) = host_cmd(&["pw-top", "-b", "-n", "2"], Duration::from_secs(4)) else {
        return (Vec::new(), None, None, 0);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("S ") && l.contains("QUANT"))
        .map(|i| i + 1)
        .unwrap_or(0);

    let mut rows = Vec::with_capacity(16);
    let (mut dr_rate, mut dr_quant, mut total_err) = (None, None, 0u64);

    for line in &lines[start..] {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 9 {
            continue;
        }
        let status = p[0].chars().next().unwrap_or('?');
        let err: u64 = p[8].parse().unwrap_or(0);
        total_err += err;

        let (fmt, name) = if p.len() >= 12 && p[10].parse::<u32>().is_ok() {
            (format!("{} {}c", p[9], p[10]), p[12..].join(" "))
        } else {
            ("-".to_string(), p[9..].join(" "))
        };

        if status == 'R' {
            if let Ok(q) = p[2].parse::<u32>() {
                if q > 0 {
                    dr_quant = Some(q);
                    dr_rate = p[3].parse().ok();
                }
            }
        }

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
            name,
            wait_us: parse_time_us(p[4]),
        });
    }
    (rows, dr_rate, dr_quant, total_err)
}

fn parse_pct(s: &str) -> f32 {
    let cleaned = s.trim_end_matches('%');
    cleaned
        .parse::<f32>()
        .map(|v| {
            if v < 1.0 && !s.contains('%') {
                v * 100.0
            } else {
                v
            }
        })
        .unwrap_or(0.0)
}
