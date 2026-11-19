use std::io::Read;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Real stereo audio level telemetry measured passively from PipeWire's sink monitor.
#[derive(Debug, Clone, Copy, Default)]
pub struct VuLevels {
    /// Normalized RMS level (0.0 = <= -60 dBFS, 1.0 = 0 dBFS).
    pub left: f64,
    pub right: f64,
    /// Peak-hold normalized level (0.0..=1.0).
    pub peak_l: f64,
    pub peak_r: f64,
    /// True if digital full-scale clipping was detected recently.
    pub clip_l: bool,
    pub clip_r: bool,
    /// Peak values in dBFS for optional tooltip/numeric readout.
    pub db_l: f64,
    pub db_r: f64,
    /// True while the passive monitor capture stream is actively delivering samples.
    pub active: bool,
}

const MIN_DB: f64 = -60.0;
const FULL_SCALE: f64 = 32768.0;
const CLIP_THRESHOLD: i16 = 32766;

/// Convert a linear amplitude in `[0.0, 1.0]` to dBFS (clamped at `MIN_DB`).
pub fn linear_to_db(amp: f64) -> f64 {
    if amp <= 1e-6 {
        MIN_DB
    } else {
        (20.0 * amp.log10()).clamp(MIN_DB, 0.0)
    }
}

/// Normalize dBFS (`-60..0`) into `[0.0, 1.0]` for bar rendering.
pub fn db_to_meter(db: f64) -> f64 {
    ((db - MIN_DB) / (-MIN_DB)).clamp(0.0, 1.0)
}

/// Analyze a chunk of raw interleaved S16_LE stereo PCM bytes.
/// Returns `(rms_l, rms_r, peak_db_l, peak_db_r, clip_l, clip_r)` where
/// `rms_*` are normalized meter positions in `[0.0, 1.0]`.
pub fn analyze_s16le_stereo(bytes: &[u8]) -> (f64, f64, f64, f64, bool, bool) {
    let frames = bytes.len() / 4;
    if frames == 0 {
        return (0.0, 0.0, MIN_DB, MIN_DB, false, false);
    }

    let mut sum_sq_l = 0.0f64;
    let mut sum_sq_r = 0.0f64;
    let mut max_l = 0i32;
    let mut max_r = 0i32;
    let mut clip_l = false;
    let mut clip_r = false;

    for chunk in bytes.as_chunks::<4>().0 {
        let l = i16::from_le_bytes([chunk[0], chunk[1]]);
        let r = i16::from_le_bytes([chunk[2], chunk[3]]);
        let al = (l as i32).abs();
        let ar = (r as i32).abs();

        if al >= CLIP_THRESHOLD as i32 {
            clip_l = true;
        }
        if ar >= CLIP_THRESHOLD as i32 {
            clip_r = true;
        }
        if al > max_l {
            max_l = al;
        }
        if ar > max_r {
            max_r = ar;
        }

        let nl = al as f64 / FULL_SCALE;
        let nr = ar as f64 / FULL_SCALE;
        sum_sq_l += nl * nl;
        sum_sq_r += nr * nr;
    }

    let rms_l = (sum_sq_l / frames as f64).sqrt();
    let rms_r = (sum_sq_r / frames as f64).sqrt();
    let peak_db_l = linear_to_db(max_l as f64 / FULL_SCALE);
    let peak_db_r = linear_to_db(max_r as f64 / FULL_SCALE);

    (
        db_to_meter(linear_to_db(rms_l)),
        db_to_meter(linear_to_db(rms_r)),
        peak_db_l,
        peak_db_r,
        clip_l,
        clip_r,
    )
}

/// Real stereo VU meter backed by a passive PipeWire sink monitor (`pw-record`).
///
/// Uses `node.passive=true` and `stream.capture.sink=true` so it NEVER wakes an
/// idle DAC or pins the PipeWire hardware sample rate, and names itself
/// `hifi-verify-vu` so telemetry excludes it from DSP/playback detection.
#[derive(Clone)]
pub struct VuMeter {
    levels: Arc<Mutex<VuLevels>>,
    running: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl VuMeter {
    pub fn new() -> Self {
        Self {
            levels: Arc::new(Mutex::new(VuLevels {
                db_l: MIN_DB,
                db_r: MIN_DB,
                ..Default::default()
            })),
            running: Arc::new(AtomicBool::new(false)),
            unavailable: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> VuLevels {
        self.levels.lock().map(|g| *g).unwrap_or_default()
    }

    /// Decay peak hold smoothly at the UI animation frame rate (~30 Hz).
    pub fn decay_tick(&self) {
        if let Ok(mut lv) = self.levels.lock() {
            if !lv.active {
                lv.left = (lv.left - 0.06).max(0.0);
                lv.right = (lv.right - 0.06).max(0.0);
                lv.peak_l = (lv.peak_l - 0.03).max(0.0);
                lv.peak_r = (lv.peak_r - 0.03).max(0.0);
                lv.clip_l = false;
                lv.clip_r = false;
            } else {
                lv.peak_l = (lv.peak_l - 0.012).max(lv.left);
                lv.peak_r = (lv.peak_r - 0.012).max(lv.right);
            }
        }
    }

    pub fn start(&self) {
        if self.unavailable.load(Ordering::Relaxed) || self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let levels = self.levels.clone();
        let running = self.running.clone();
        let unavailable = self.unavailable.clone();
        let child_slot = self.child.clone();

        std::thread::spawn(move || {
            let mut child = match crate::host::host_spawn_piped(&[
                "pw-record",
                "--raw",
                "--format",
                "s16",
                "--channels",
                "2",
                "--rate",
                "48000",
                "--latency",
                "30ms",
                "-P",
                "{ node.name=\"hifi-verify-vu\" node.passive=true stream.capture.sink=true }",
                "-",
            ]) {
                Ok(c) => c,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        unavailable.store(true, Ordering::Relaxed);
                    }
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let Some(mut stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();
                running.store(false, Ordering::SeqCst);
                return;
            };

            if let Ok(mut slot) = child_slot.lock() {
                *slot = Some(child);
            }

            // 4096 bytes = 1024 stereo S16_LE frames ≈ 21.3 ms at 48 kHz.
            let mut buf = [0u8; 4096];
            while running.load(Ordering::Relaxed) {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let (rms_l, rms_r, db_l, db_r, clip_l, clip_r) =
                            analyze_s16le_stereo(&buf[..n]);
                        let pk_l = db_to_meter(db_l);
                        let pk_r = db_to_meter(db_r);
                        if let Ok(mut lv) = levels.lock() {
                            // Fast attack, smooth ballistic release.
                            lv.left = if rms_l > lv.left {
                                rms_l
                            } else {
                                lv.left * 0.72 + rms_l * 0.28
                            };
                            lv.right = if rms_r > lv.right {
                                rms_r
                            } else {
                                lv.right * 0.72 + rms_r * 0.28
                            };
                            lv.peak_l = lv.peak_l.max(pk_l);
                            lv.peak_r = lv.peak_r.max(pk_r);
                            lv.db_l = db_l;
                            lv.db_r = db_r;
                            lv.clip_l = clip_l;
                            lv.clip_r = clip_r;
                            lv.active = true;
                        }
                    }
                    Err(_) => break,
                }
            }

            if let Ok(mut lv) = levels.lock() {
                lv.active = false;
            }
            if let Ok(mut slot) = child_slot.lock() {
                if let Some(mut c) = slot.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
            running.store(false, Ordering::SeqCst);
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut c) = slot.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if let Ok(mut lv) = self.levels.lock() {
            lv.active = false;
            lv.clip_l = false;
            lv.clip_r = false;
        }
    }
}

impl Drop for VuMeter {
    fn drop(&mut self) {
        if Arc::strong_count(&self.child) == 1 {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_measures_zero_and_min_db() {
        let silence = vec![0u8; 4096];
        let (l, r, db_l, db_r, clip_l, clip_r) = analyze_s16le_stereo(&silence);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(db_l, MIN_DB);
        assert_eq!(db_r, MIN_DB);
        assert!(!clip_l && !clip_r);
    }

    #[test]
    fn measures_independent_left_and_right_channels_and_clipping() {
        let mut buf = Vec::with_capacity(4 * 256);
        for _ in 0..256 {
            let left: i16 = 32767; // 0 dBFS full-scale clip
            let right: i16 = 4096; // ~-18 dBFS
            buf.extend_from_slice(&left.to_le_bytes());
            buf.extend_from_slice(&right.to_le_bytes());
        }
        let (l, r, db_l, db_r, clip_l, clip_r) = analyze_s16le_stereo(&buf);
        assert!(l > 0.99);
        assert!(r > 0.65 && r < 0.75);
        assert!(db_l > -0.1);
        assert!(db_r < -17.0 && db_r > -19.0);
        assert!(clip_l);
        assert!(!clip_r);
    }
}
