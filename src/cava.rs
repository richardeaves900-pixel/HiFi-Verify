use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub fn spawn_cava_worker(bands: Arc<Mutex<Vec<f64>>>) {
    std::thread::spawn(move || {
        let config = "[general]\nbars = 32\n\n[output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = 100\nchannels = mono\n";
        let config_path = "/tmp/hfv_cava.conf";
        std::fs::write(config_path, config).unwrap();

        let in_flatpak = std::path::Path::new("/.flatpak-info").exists();
        let mut cmd = if in_flatpak {
            let mut c = Command::new("flatpak-spawn");
            c.arg("--host").arg("cava").arg("-p").arg(config_path);
            c
        } else {
            let mut c = Command::new("cava");
            c.arg("-p").arg(config_path);
            c
        };

        let mut child = match cmd.stdout(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => return,
        };

        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        for line in reader.lines() {
            if let Ok(l) = line {
                // Cava outputs strings like "10;20;15;..." followed by a semicolon
                let parts: Vec<f64> = l.trim_end_matches(';')
                    .split(';')
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                
                if parts.len() == 32 {
                    if let Ok(mut b) = bands.lock() {
                        *b = parts;
                    }
                }
            }
        }
    });
}
