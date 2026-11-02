# HiFi Verify

**A native, Flatpak-first PipeWire verification suite for audiophiles.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#)

HiFi Verify is a ground-up Rust rewrite of an earlier Python audio verifier. It is designed to answer one question definitively: **"Is my audio bit-perfect right now, and if not, why?"**

Unlike standard system monitors, HiFi Verify reads directly from the hardware (`/proc/asound`), parses `pw-top` latency metrics, and visually maps the PipeWire signal path. It is distributed as a Flatpak, using secure host-spawning to read hardware metrics without breaking sandbox isolation.

---

## ✨ Features

* **Signal Path Matrix:** Instantly see if EasyEffects (or any other DSP) is intercepting your audio, and view the exact format your ALSA hardware is receiving.
* **Hardware Direct:** Reads `/proc/asound` directly to verify the true DAC format (e.g., `S32_LE @ 48.0kHz`), bypassing PipeWire's abstractions.
* **Latency Jitter Graphing:** Plots real-time `pw-top` WAIT metrics to detect buffer underruns before you hear them.
* **CLI & JSON Mode:** Sub-100ms CLI commands (`--health`, `--audit`, `--status`) designed for scripting, status bars, and automation.
* **KDE Plasma Widget:** A custom plasmoid that sits in your system tray and displays status via color coding: **Green** (bit-perfect), **Red** (compromised/processing), or **Gray** (idle).
* **Sandbox Safe:** Fully compliant Flatpak architecture using `flatpak-spawn --host` to bridge the sandbox for hardware telemetry.

---

## 📸 Screenshot

<img width="2112" height="1187" alt="image" src="https://github.com/user-attachments/assets/ebc356b9-36ce-4eab-9318-4eb5b90f3fda" />



---

## 🚀 Installation

### Option 1: Flatpak (Recommended)

Since the app is not yet on Flathub, build and install it locally:

1. **Install required runtimes:**

   ```bash
   flatpak install flathub org.gnome.Platform//49 org.freedesktop.Sdk.Extension.rust-stable//25.08
   ```

2. **Clone the repository and build:**

   ```bash
   git clone [https://github.com/richardeaves900-pixel/HiFi-Verify.git](https://github.com/richardeaves900-pixel/HiFi-Verify.git)
   cd HiFi-Verify/flatpak
   flatpak-builder --user --install --force-clean build-dir org.cachyos.HiFiVerify.yaml
   ```

> **Note:** If `cargo-sources.json` is out of date, run:
> ```bash
> python3 flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
> ```

---

### Option 2: Build from Source (Host)

**Prerequisites:** Rust (1.92+), GTK4 development headers, and PipeWire.

1. **Clone the repository:**

   ```bash
   git clone [https://github.com/richardeaves900-pixel/HiFi-Verify.git](https://github.com/richardeaves900-pixel/HiFi-Verify.git)
   cd HiFi-Verify
   ```

2. **Build and run:**

   ```bash
   cargo build --release
   ./target/release/hifi-verify --gui
   ```

---

🎧 HiFi Verify: Full Feature List

HiFi Verify is a native, Flatpak-first PipeWire verification suite for audiophiles. It acts as a transparent window into your Linux audio stack, reading directly from hardware to prove exactly what your DAC is receiving.
🔬 Core Audio Verification & Telemetry

    Hardware Direct Reading: Parses /proc/asound/card*/pcm*p/sub*/hw_params to bypass PipeWire's abstractions and read the exact ALSA format (e.g., S32_LE) and sample rate your DAC is receiving.

    ALSA Bit-Perfect Verification: Dynamically compares the source stream's sample rate against the ALSA hardware rate. Displays a definitive ✓ ALSA Bit-Perfect or ⚠ ALSA Resampled indicator directly in the UI.

    DSP Detection: Scans the PipeWire graph to detect active processing nodes (like EasyEffects, Carla, or JamesDSP) that compromise bit-perfect playback.

    Signal Path Matrix: A custom Cairo-rendered flowchart that visually maps the audio path from the Source App ➔ through any DSP ➔ to the final ALSA DAC Bus.

    Latency Jitter Graphing: Plots real-time pw-top WAIT metrics (in microseconds) to detect buffer underruns and system jitter before they become audible.

    Hardware XRUN Tracking: Monitors per-node dropped frames (XRUNs) and surfaces them instantly on the dashboard.

💻 Graphical User Interface (GTK4)

    Native Dark-Mode Dashboard: A sleek, monospace-themed UI designed for Linux power users.

    DAC Capability Viewer: Queries your hardware to display supported sample rates (e.g., 8kHz to 384kHz), human-readable PCM formats (e.g., Signed 24-bit LE), and Native DSD transport capabilities (e.g., 32-bit BE).

    Cava Audio Spectrum: A built-in, zero-latency 32-band frequency visualizer that integrates the cava backend. Toggle seamlessly between the Signal Path Matrix and the spectrum with one click.

    Hardware Volume Slider: A dedicated slider that uses wpctl to adjust the hardware DAC volume (avoiding digital attenuation), complete with real-time sync to your system volume keys.

    Live "Now Playing": Displays the active source app, track name, and codec directly in the top bar.

    Quick Action Buttons: One-click launches for EasyEffects, KDE Audio Settings, and qpwgraph.

    XRUN History Viewer: A popup dialog that reads a local SQLite database to show you the exact timestamps and frequency of past audio dropouts.

🎛️ System & Media Control

    MPRIS Integration: Full support for standard Linux media keys.

    Playback Controls: ⏮ Previous, ⏯ Play/Pause, and ⏭ Next buttons built directly into the GUI top bar and the System Tray.

    Desktop Notifications: Fires a KDE desktop notification instantly if a DSP (like EasyEffects) is activated, warning you that bit-perfect playback has been compromised.

    Persistent XRUN Logging: Silently logs XRUN events to ~/.config/hifi-verify/history.db via SQLite for post-session diagnostics.

🖥️ KDE Plasma System Tray (StatusNotifierItem)

    Dynamic Color-Coded Icon: A procedurally generated tray icon that turns Emerald Green (Bit-Perfect), Ruby Red (Compromised/DSP Active), or Slate Grey (Standby).

    Scroll-Wheel Volume: Hover over the tray icon and scroll your mouse wheel to adjust hardware volume by 5% increments.

    Live Telemetry Tooltip: Hovering over the icon shows the active app, codec, DAC hardware, and DSP filters.

    Rich Context Menu: Right-click to access media controls, volume controls, mute toggle, window toggle, and app exit.

    KDE Plasma Widget (QML): A standalone Plasmoid that polls the CLI via a background timer to display a simple red/green dot on your panel.

🌐 Network Verification

    SSDP / UPnP Discovery: Scans your local network for media renderers (e.g., network streamers, smart TVs, Raspberry Pis running UPnP).

    Device Resolution: Fetches the device description XML from discovered renderers to display their human-readable names (e.g., "Living Room Streamer") alongside their IP addresses.

⌨️ Command Line Interface (CLI)

    --health: Sub-100ms one-line summary of the current audio state.

    --audit: Dumps a comprehensive JSON report of the hardware params, active nodes, and warnings.

    --audit --fast: Skips pw-top parsing for a near-instant audit.

    --hw: Lists detailed ALSA hardware parameters for all playback PCMs.

    --status: Outputs strict JSON for the KDE widget ({"state": "green", "reason": "Bit-Perfect"}).

    --network-scan: Triggers the SSDP scanner directly from the terminal.

    --history: Prints your historical XRUN logs.

    --ramp-test: Runs a null-test by generating a test WAV, playing it, and capturing it via a loopback to mathematically prove bit-perfect routing.

    --wireplumber: Generates a persistent WirePlumber rule matching your current hardware rate/quantum.

🏗️ Architecture & Packaging

    Rust Native Performance: ~40ms CLI startup time and ~40MB RAM footprint at idle.

    Flatpak Sandbox Bridge: Safely runs inside a Flatpak sandbox while using a secure flatpak-spawn --host bridge to read host /proc files and execute pw-top.

    Zero Host Dependencies: Unlike the previous Python version, the Rust binary requires zero host runtimes or external scripts.

    Multi-threaded Event Loop: Uses std::sync::mpsc channels to keep the GTK4 UI rendering at 30fps while background threads poll hardware metrics every 500ms.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## 🙏 Acknowledgements & Disclosures

* **[gtk-rs](https://gtk-rs.org/)** – Used for building the native Linux user interface.
* **pw-top Visualizer** – Inspired by the original Python prototype.
* **AI Disclosure** – This project was developed with assistance from AI models (Gemini and DeepSeek).
