use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::{Path, PathBuf};
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

/// Discover a loopback/null sink monitor via `pactl list short sources`.
/// Returns `(monitor_source_name, sink_name)`.
fn parse_monitor_source(pactl_out: &str) -> Option<(String, String)> {
    let mut fallback: Option<(String, String)> = None;
    for line in pactl_out.lines() {
        let mut cols = line.split_whitespace();
        let _idx = cols.next()?;
        let Some(name) = cols.next() else {
            continue;
        };
        let Some(sink) = name.strip_suffix(".monitor") else {
            continue;
        };
        let pair = (name.to_string(), sink.to_string());
        let lower = name.to_ascii_lowercase();
        if lower.contains("bp_test") || lower.contains("null") || lower.contains("loopback") {
            return Some(pair);
        }
        if fallback.is_none() {
            fallback = Some(pair);
        }
    }
    fallback
}

fn find_monitor() -> Option<(String, String)> {
    let out = crate::host::host_cmd(
        &["pactl", "list", "short", "sources"],
        Duration::from_secs(3),
    )
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_monitor_source(&text)
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

    let Some((monitor_src, target_sink)) = find_monitor() else {
        return serde_json::json!({
            "status": "skipped",
            "detail": "No loopback/monitor source found. Create one with: pactl load-module module-null-sink sink_name=bp_test",
            "wav": wav.display().to_string()
        });
    };

    let cap = dir.join(format!("capture-{}.wav", rate));
    let _ = std::fs::remove_file(&cap);

    let rate_arg = format!("--rate={}", rate);
    let wav_str = wav.to_string_lossy().into_owned();

    // Stream raw S16_LE PCM from parec stdout and write a guaranteed-valid WAV
    // header via `hound` after stopping parec (avoiding truncated WAV headers on kill).
    let mut recorder = match crate::host::host_spawn_piped(&[
        "parec",
        "--device",
        &monitor_src,
        "--format=s16le",
        &rate_arg,
        "--channels=1",
        "--raw",
    ]) {
        Ok(c) => c,
        Err(_) => {
            return serde_json::json!({
                "status": "error",
                "detail": "parec not available on host",
                "wav": wav_str
            });
        }
    };

    let stdout = recorder.stdout.take();
    let reader_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::with_capacity(rate as usize * 4);
        if let Some(mut s) = stdout {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
        }
        bytes
    });

    std::thread::sleep(Duration::from_millis(120));

    let play_res = crate::host::host_cmd(
        &["paplay", "--device", &target_sink, &wav_str],
        Duration::from_secs(6),
    );

    std::thread::sleep(Duration::from_millis(250));

    let _ = recorder.kill();
    let _ = recorder.wait();
    let raw_bytes = reader_handle.join().unwrap_or_default();

    if play_res.is_err() {
        return serde_json::json!({
            "status": "error",
            "detail": "paplay failed or timed out on host",
            "wav": wav_str
        });
    }

    if let Err(err) = write_raw_s16le_to_wav(&cap, rate, &raw_bytes) {
        return serde_json::json!({
            "status": "error",
            "detail": format!("failed to write capture WAV: {err}"),
            "wav": wav_str
        });
    }

    let mut verified = crate::pcm_verify::verify_wav(&wav, &cap);
    if let Some(obj) = verified.as_object_mut() {
        obj.insert("monitor".into(), serde_json::json!(monitor_src));
        obj.insert("sink".into(), serde_json::json!(target_sink));
        obj.insert("wav".into(), serde_json::json!(wav_str));
    }
    verified
}

fn write_raw_s16le_to_wav(path: &Path, rate: u32, raw_bytes: &[u8]) -> Result<(), String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for pair in raw_bytes.as_chunks::<2>().0 {
        let sample = i16::from_le_bytes(*pair);
        writer.write_sample(sample).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_loopback_or_null_monitor_over_hardware_monitor() {
        let output = "\
44\talsa_output.usb-Fiio.analog-stereo.monitor\tPipeWire\ts32le 2ch 96000Hz\tRUNNING
88\tbp_test.monitor\tPipeWire\ts16le 2ch 48000Hz\tIDLE
";
        assert_eq!(
            parse_monitor_source(output),
            Some(("bp_test.monitor".into(), "bp_test".into()))
        );
    }

    #[test]
    fn falls_back_to_first_monitor_source() {
        let output = "\
44\talsa_output.usb-Fiio.analog-stereo.monitor\tPipeWire\ts32le 2ch 96000Hz\tRUNNING
";
        assert_eq!(
            parse_monitor_source(output),
            Some((
                "alsa_output.usb-Fiio.analog-stereo.monitor".into(),
                "alsa_output.usb-Fiio.analog-stereo".into()
            ))
        );
    }

    #[test]
    fn raw_capture_roundtrips_through_pcm_verify_with_lead_in() {
        let dir = std::env::temp_dir().join("hifi-verify-ramp-unit");
        let _ = std::fs::create_dir_all(&dir);
        let ref_wav = dir.join("ref.wav");
        let cap_wav = dir.join("cap.wav");
        create_ramp(&ref_wav, 48_000).unwrap();

        let mut reader = hound::WavReader::open(&ref_wav).unwrap();
        let mut raw = vec![0u8; 64]; // 32 samples of lead-in silence
        for sample in reader.samples::<i16>() {
            raw.extend_from_slice(&sample.unwrap().to_le_bytes());
        }
        write_raw_s16le_to_wav(&cap_wav, 48_000, &raw).unwrap();

        let res = crate::pcm_verify::verify_wav(&ref_wav, &cap_wav);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(res["status"], "pass");
        assert_eq!(res["capture_lead_in_frames"], 32);
    }
}
