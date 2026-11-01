HiFi Verify

**A native, Flatpak-first PipeWire verification suite for audiophiles.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)]()

HiFi Verify is a ground-up Rust rewrite of an earlier Python audio verifier. It is designed to answer one question definitively: **"Is my audio bit-perfect right now, and if not, why?"**

Unlike standard system monitors, HiFi Verify reads directly from the hardware (`/proc/asound`), parses the `pw-top` latency metrics, and visually maps the PipeWire signal path. It is distributed as a Flatpak, using secure host-spawning to read hardware metrics without breaking sandbox isolation.

## ✨ Features

*   **Signal Path Matrix:** Instantly see if EasyEffects (or any other DSP) is intercepting your audio, and exactly what format your ALSA hardware is receiving.
*   **Hardware Direct:** Reads `/proc/asound` directly to verify the true DAC format (e.g., `S32_LE @ 48.0kHz`), bypassing PipeWire's abstractions.
*   **Latency Jitter Graphing:** Plots real-time `pw-top` WAIT metrics to detect buffer underruns before you hear them.
*   **CLI & JSON Mode:** Sub-100ms CLI commands (`--health`, `--audit`, `--status`) perfect for scripting, status bars, and automation.
*   **KDE Plasma Widget:** A custom plasmoid that sits in your system tray and turns **Green** (bit-perfect), **Red** (compromised/processing), or **Gray** (idle).
*   **Sandbox Safe:** Fully compliant Flatpak architecture using `flatpak-spawn --host` to bridge the sandbox for hardware telemetry.

Screenshot
<img width="1733" height="1155" alt="image" src="https://github.com/user-attachments/assets/cc3cf694-0a23-4b1c-9d29-0e8c1809d914" />


---
🚀 Installation

### Option 1: Flatpak (Recommended)
Since the app is not yet on Flathub, you can build and install it locally:

1. Ensure you have the required runtimes:
   ```bash
   flatpak install flathub org.gnome.Platform//49 org.freedesktop.Sdk.Extension.rust-stable//25.08

2C one the repo and build
```bash
git clone https://github.com/richardeaves900-pixel/HiFi-Verify.git
cd HiFi-Verify/flatpak
flatpak-builder --user --install --force-clean build-dir org.cachyos.HiFiVerify.yaml
```

Note: If cargo-sources.json is out of date, run python3 flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json first.

Option 2: Build from Source (Host)

Requires Rust (1.92+), GTK4 development headers, and PipeWire.

Step 1: Clone the repository:
```bash
git clone https://github.com/richardeaves900-pixel/HiFi-Verify.git
cd HiFi-Verify
```
Step 2: Build and run:
```bash
cargo build --release
./target/release/hifi-verify --gui
```


