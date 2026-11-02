use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn create_ramp(path: &Path, rate: u32) -> Result<(), String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut w = WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let n = rate as i64;
    let max = i16::MAX as i64;
    for i in 0..n {
        let sample = ((i * max) / n) as i16;
        w.write_sample(sample).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

fn find_monitor() -> Option<String> {
    let out = crate::host::host_cmd(&["pw-cli", "ls", "Node"], Duration::from_secs(3)).ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains("monitor") || l.contains("loopback"))
        .and_then(|l| l.split("id ").nth(1))
        .and_then(|s| s.split_whitespace().next())
        .map(str::to_string)
}

pub fn run(rate: u32) -> serde_json::Value {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hifi-verify/ramp-test");
    std::fs::create_dir_all(&dir).ok();
    let wav = dir.join(format!("ramp-{}.wav", rate));

    if let Err(e) = create_ramp(&wav, rate) {
        return serde_json::json!({ "status": "error", "detail": e });
    }

    let Some(src) = find_monitor() else {
        return serde_json::json!({
            "status": "skipped",
            "detail": "No loopback/monitor source found. Create one with: pactl load-module module-null-sink sink_name=bp_test",
            "wav": wav.display().to_string()
        });
    };

    let cap = dir.join(format!("capture-{}.raw", rate));
    let _ = std::fs::remove_file(&cap);

    let mut recorder = Command::new("parec")
        .args([
            "--device",
            &src,
            "--format=s16le",
            &format!("--rate={}", rate),
            "--channels=1",
            "--raw",
            "--file-format=raw",
        ])
        .arg(&cap)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();

    if recorder.is_none() {
        return serde_json::json!({ "status": "error", "detail": "parec not available" });
    }

    std::thread::sleep(Duration::from_millis(100));

    let _ = Command::new("paplay")
        .arg(&wav)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    std::thread::sleep(Duration::from_millis(200));

    if let Some(mut r) = recorder.take() {
        let _ = r.kill();
        let _ = r.wait();
    }

    let Ok(ref_bytes) = std::fs::read(&wav) else {
        return serde_json::json!({ "status": "error", "detail": "read reference failed" });
    };
    let cap_bytes = std::fs::read(&cap).unwrap_or_default();

    if ref_bytes.len() < 44 {
        return serde_json::json!({ "status": "error", "detail": "reference WAV too small" });
    }
    let reference = &ref_bytes[44..];
    let mut offset = 0;
    while offset + 2 <= cap_bytes.len() && cap_bytes[offset..offset + 2] == [0, 0] {
        offset += 2;
    }
    let aligned = &cap_bytes[offset..(offset + reference.len()).min(cap_bytes.len())];

    if aligned == reference {
        serde_json::json!({ "status": "pass", "detail": "captured samples match source ramp" })
    } else if aligned.is_empty() {
        serde_json::json!({ "status": "skipped", "detail": "capture was empty or silent" })
    } else {
        let diffs = aligned
            .iter()
            .zip(reference)
            .filter(|(a, b)| a != b)
            .count();
        serde_json::json!({ "status": "fail", "detail": format!("{} byte(s) differ", diffs) })
    }
}
