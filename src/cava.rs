use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CAVA_CONFIG: &str = "\
[general]
bars = 32
framerate = 30

[output]
method = raw
raw_target = /dev/stdout
data_format = ascii
ascii_max_range = 100
channels = mono
";

/// Lazy, clean-lifecycle controller for the `cava` 32-band spectrum backend.
/// Starts only when the user enables the Audio Spectrum view and terminates
/// its child process when hidden or on application exit.
#[derive(Clone)]
pub struct CavaController {
    bands: Arc<Mutex<Vec<f64>>>,
    running: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl CavaController {
    pub fn new() -> Self {
        Self {
            bands: Arc::new(Mutex::new(vec![0.0; 32])),
            running: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn bands(&self) -> Arc<Mutex<Vec<f64>>> {
        self.bands.clone()
    }

    pub fn start(&self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let bands = self.bands.clone();
        let running = self.running.clone();
        let child_slot = self.child.clone();

        std::thread::spawn(move || {
            // Create a host-side temporary config via `sh -c` so this works
            // identically both natively and inside a Flatpak sandbox (where
            // `/tmp` is private to the container).
            let script = "cfg=$(mktemp /tmp/hfv_cava.XXXXXX) && \
                          trap 'rm -f \"$cfg\"' EXIT INT TERM && \
                          printf '%s' \"$1\" > \"$cfg\" && \
                          exec cava -p \"$cfg\"";

            let Ok(mut child) =
                crate::host::host_spawn_piped(&["sh", "-c", script, "--", CAVA_CONFIG])
            else {
                running.store(false, Ordering::SeqCst);
                return;
            };

            let Some(stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();
                running.store(false, Ordering::SeqCst);
                return;
            };

            if let Ok(mut slot) = child_slot.lock() {
                *slot = Some(child);
            }

            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                let parts: Vec<f64> = line
                    .trim_end_matches(';')
                    .split(';')
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                if parts.len() == 32 {
                    if let Ok(mut b) = bands.lock() {
                        *b = parts;
                    }
                }
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
        if let Ok(mut b) = self.bands.lock() {
            b.fill(0.0);
        }
    }
}

impl Drop for CavaController {
    fn drop(&mut self) {
        // Only terminate when the final controller reference is dropped.
        if Arc::strong_count(&self.child) == 1 {
            self.stop();
        }
    }
}
