//! Strict PCM reference/capture comparison at a known capture point.
//! This does NOT verify DAC analogue output or unrelated streaming-service sources.
use hound::{SampleFormat, WavReader};
use serde_json::{json, Value};
use std::path::Path;

const MAX_FRAMES: usize = 30 * 192_000; // bounded memory and work: first 30 seconds

#[derive(Debug)]
struct Pcm {
    rate: u32,
    channels: u16,
    bits: u16,
    frames: usize,
    samples: Vec<i64>,
}

fn read_pcm(path: &Path) -> Result<Pcm, String> {
    let mut reader = WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_format != SampleFormat::Int || !matches!(spec.bits_per_sample, 16 | 24 | 32) {
        return Err("Only 16-, 24- or 32-bit integer PCM WAV is supported; no float/codec conversion is assumed".into());
    }
    if spec.channels == 0 || spec.channels > 8 || spec.sample_rate == 0 {
        return Err("Invalid WAV channels or sample rate".into());
    }
    let count_limit = MAX_FRAMES.saturating_mul(spec.channels as usize);
    let mut samples = Vec::new();
    for sample in reader.samples::<i32>().take(count_limit) {
        samples.push(i64::from(sample.map_err(|e| e.to_string())?));
    }
    let channels = spec.channels as usize;
    samples.truncate(samples.len() / channels * channels);
    let frames = samples.len() / channels;
    Ok(Pcm {
        rate: spec.sample_rate,
        channels: spec.channels,
        bits: spec.bits_per_sample,
        frames,
        samples,
    })
}

fn normalize_sample(value: i64, reference_bits: u16, capture_bits: u16) -> Option<i64> {
    if capture_bits < reference_bits {
        return None;
    }
    let shift = (capture_bits - reference_bits) as u32;
    let factor = 1i64 << shift;
    if value % factor != 0 {
        return None;
    }
    Some(value / factor)
}

fn compare(reference: &Pcm, capture: &Pcm) -> Value {
    if reference.rate != capture.rate || reference.channels != capture.channels {
        return json!({"status":"fail", "verification":"not_bit_perfect", "detail": format!(
            "WAV formats differ: source {} Hz/{} channels; capture {} Hz/{} channels",
            reference.rate, reference.channels, capture.rate, capture.channels)});
    }
    if capture.bits < reference.bits {
        return json!({"status":"fail", "verification":"not_bit_perfect",
            "detail":"Capture has fewer valid PCM bits than the reference"});
    }
    if reference.frames < reference.rate as usize / 2 {
        return json!({"status":"inconclusive", "verification":"unverified",
            "detail":"Reference needs at least half a second of audio"});
    }
    if reference.samples.iter().all(|v| *v == 0) {
        return json!({"status":"inconclusive", "verification":"unverified",
            "detail":"All-silent reference cannot establish bit-perfect playback"});
    }
    let channels = reference.channels as usize;
    if capture.frames < reference.frames {
        return json!({"status":"inconclusive", "verification":"unverified",
            "detail":"Captured audio is shorter than reference"});
    }
    // Compare full frames; tolerate recording lead-in silence and latency, but never
    // tolerate altered samples, rate conversion, dropped samples or gain changes.
    // First compare a 64-frame probe, then all frames. Exhaustive bounded scan.
    let search_to =
        (capture.frames - reference.frames).min((reference.rate as usize).saturating_mul(10));
    let probe = 64usize.min(reference.frames);
    for start in 0..=search_to {
        let mut probe_matches = true;
        for i in 0..probe * channels {
            match normalize_sample(
                capture.samples[start * channels + i],
                reference.bits,
                capture.bits,
            ) {
                Some(s) if s == reference.samples[i] => (),
                _ => {
                    probe_matches = false;
                    break;
                }
            }
        }
        if !probe_matches {
            continue;
        }
        let matches = reference.samples.iter().enumerate().all(|(i, &s)| {
            normalize_sample(
                capture.samples[start * channels + i],
                reference.bits,
                capture.bits,
            ) == Some(s)
        });
        if matches {
            return json!({"status":"pass", "verification":"pcm_match_at_capture_point",
                "detail":"All reference PCM samples match a contiguous capture segment exactly (lossless width expansion permitted)",
                "rate":reference.rate, "channels":reference.channels,
                "reference_bits":reference.bits, "capture_bits":capture.bits,
                "matched_frames":reference.frames, "capture_lead_in_frames":start,
                "scope":"Proves only the selected digital capture point for this known reference; not DAC analogue output or other streaming tracks"});
        }
    }
    json!({"status":"fail", "verification":"not_bit_perfect_at_capture_point",
        "detail":"No contiguous exact PCM match found in capture (within first 10 seconds of lead-in). Check playback, recording point, volume and sample-rate conversion"})
}

pub fn verify_wav(reference: &Path, capture: &Path) -> Value {
    match (read_pcm(reference), read_pcm(capture)) {
        (Ok(r), Ok(c)) => compare(&r, &c),
        (Err(e), _) | (_, Err(e)) => {
            json!({"status":"error", "verification":"unverified", "detail":e})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pcm(bits: u16, rate: u32, samples: Vec<i64>) -> Pcm {
        Pcm {
            rate,
            bits,
            channels: 1,
            frames: samples.len(),
            samples,
        }
    }
    fn signal() -> Vec<i64> {
        (0..48_000)
            .map(|i| ((i as i64 * 79) % 32767) - 16384)
            .collect()
    }
    #[test]
    fn exact_match_with_capture_lead_in() {
        let source = pcm(16, 48_000, signal());
        let mut capture = vec![0; 80];
        capture.extend_from_slice(&source.samples);
        assert_eq!(
            compare(&source, &pcm(16, 48_000, capture))["status"],
            "pass"
        );
    }
    #[test]
    fn lossless_16_to_32_widening_passes() {
        let source = pcm(16, 48_000, signal());
        let capture = pcm(
            32,
            48_000,
            source.samples.iter().map(|&x| x << 16).collect(),
        );
        assert_eq!(compare(&source, &capture)["status"], "pass");
    }
    #[test]
    fn changed_sample_fails() {
        let source = pcm(16, 48_000, signal());
        let mut bad = source.samples.clone();
        bad[200] += 1;
        assert_eq!(compare(&source, &pcm(16, 48_000, bad))["status"], "fail");
    }
    #[test]
    fn rate_conversion_does_not_pass() {
        let source = pcm(16, 48_000, signal());
        assert_eq!(
            compare(&source, &pcm(16, 96_000, signal()))["status"],
            "fail"
        );
    }
    #[test]
    fn silence_is_inconclusive() {
        let source = pcm(16, 48_000, vec![0; 48_000]);
        assert_eq!(compare(&source, &source)["status"], "inconclusive");
    }
}
