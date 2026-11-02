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

<div align="center">

# 🎧 HiFi Verify
### *The Ultimate PipeWire Audio Verification Suite for Linux Audiophiles*

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Toolkit: GTK4](https://img.shields.io/badge/Toolkit-GTK4-blue.svg)](https://gtk-rs.org/)
[![Packaging: Flatpak](https://img.shields.io/badge/Packaging-Flatpak-blueviolet.svg)](https://flatpak.org/)

</div>

---

**HiFi Verify** is a high-performance, native, Flatpak-first verification suite designed specifically for Linux audiophiles. Acting as a transparent, low-overhead window into your audio stack, it reads directly from hardware to definitively prove what your DAC is receiving—ensuring true bit-perfect playback without compromise.

---

## 🔬 Core Audio Verification & Telemetry

* **Hardware Direct Reading:** Parses `/proc/asound/card*/pcm*p/sub*/hw_params` directly, bypassing PipeWire's abstractions to read the exact ALSA format (e.g., `S32_LE`) and sample rate your DAC is processing.
* **ALSA Bit-Perfect Verification:** Dynamically compares the source stream's sample rate against the ALSA hardware rate, surfacing a clear `✓ ALSA Bit-Perfect` or `⚠ ALSA Resampled` status indicator directly in the UI.
* **DSP Detection:** Scans the PipeWire graph to detect active processing nodes (such as EasyEffects, Carla, or JamesDSP) that compromise bit-perfect workflows.
* **Signal Path Matrix:** Features a custom Cairo-rendered flowchart mapping the audio path from Source App $\rightarrow$ DSP $\rightarrow$ Final ALSA DAC Bus.
* **Latency Jitter Graphing:** Plots real-time `pw-top` WAIT metrics (in microseconds) to catch buffer underruns and system jitter before they cause audible artifacts.
* **Hardware XRUN Tracking:** Monitors per-node dropped frames (XRUNs) and surfaces them instantly on the dashboard.

---

## 💻 Graphical User Interface (GTK4)

* **Native Dark-Mode Dashboard:** Sleek, monospace-themed interface tailored for Linux power users.
* **DAC Capability Viewer:** Directly queries hardware to display supported sample rates (8kHz to 384kHz), human-readable PCM formats (e.g., Signed 24-bit LE), and Native DSD transport capabilities.
* **Cava Audio Spectrum:** Built-in, zero-latency 32-band frequency visualizer integrating the `cava` backend. Toggle seamlessly between the Signal Path Matrix and the spectrum with a single click.
* **Hardware Volume Slider:** Dedicated slider utilizing `wpctl` to adjust hardware DAC volume directly (preventing digital attenuation) with real-time sync to system volume keys.
* **Live "Now Playing":** Top bar widget displaying the active source application, track name, and audio codec.
* **Quick Action Buttons:** One-click shortcuts for EasyEffects, KDE Audio Settings, and `qpwgraph`.
* **XRUN History Viewer:** Popup dialog reading a local SQLite database to display exact timestamps and frequency of past audio dropouts.

---

## 🎛️ System & Media Control

* **MPRIS Integration:** Full native support for standard Linux media keys.
* **Playback Controls:** Built-in `⏮ Previous`, `⏯ Play/Pause`, and `⏭ Next` buttons integrated directly into the GUI top bar and System Tray.
* **Desktop Notifications:** Fires an instant KDE desktop notification if a DSP (like EasyEffects) activates, warning you that bit-perfect playback has been compromised.
* **Persistent XRUN Logging:** Silently logs XRUN events to `~/.config/hifi-verify/history.db` via SQLite for comprehensive post-session diagnostics.

---

## 🖥️ KDE Plasma System Tray & Widgets (`StatusNotifierItem`)

* **Dynamic Color-Coded Icon:** Procedurally generated tray icon that changes color based on system state:
  * 🟢 **Emerald Green:** Bit-Perfect
  * 🔴 **Ruby Red:** Compromised / DSP Active
  * ⚫ **Slate Grey:** Standby
* **Scroll-Wheel Volume:** Hover over the tray icon and scroll to adjust hardware volume in precise 5% increments.
* **Live Telemetry Tooltip:** Instantly view the active app, codec, DAC hardware, and DSP filters on hover.
* **Rich Context Menu:** Right-click for quick access to media controls, volume levels, mute toggle, window visibility, and application exit.
* **KDE Plasma Widget (QML):** Standalone Plasmoid that polls the CLI via a background timer to display a minimal status indicator directly on your panel.

---

## 🌐 Network Verification

* **SSDP / UPnP Discovery:** Scans your local network for media renderers (network streamers, smart TVs, Raspberry Pis running UPnP).
* **Device Resolution:** Fetches device description XML from discovered renderers to display human-readable names (e.g., *"Living Room Streamer"*) alongside their network IP addresses.

---

## ⌨️ Command Line Interface (CLI) Reference

| Command | Description |
| :--- | :--- |
| `--health` | Sub-100ms one-line summary of the current audio state. |
| `--audit` | Dumps a comprehensive JSON report of hardware parameters, active nodes, and warnings. |
| `--audit --fast` | Skips `pw-top` parsing for a near-instant audit. |
| `--hw` | Lists detailed ALSA hardware parameters for all playback PCMs. |
| `--status` | Outputs strict JSON for the KDE widget (`{"state": "green", "reason": "Bit-Perfect"}`). |
| `--network-scan` | Triggers the SSDP network scanner directly from the terminal. |
| `--history` | Prints your historical XRUN logs. |
| `--ramp-test` | Runs a null-test by generating a test WAV, playing it, and capturing it via a loopback to mathematically prove bit-perfect routing. |
| `--wireplumber` | Generates a persistent WirePlumber rule matching your current hardware rate/quantum. |

---

## 🏗️ Architecture & Performance

* **Rust Native Performance:** Achieves ~40ms CLI startup times and maintains a ~40MB RAM footprint at idle.
* **Flatpak Sandbox Bridge:** Safely operates within a strict Flatpak sandbox while leveraging a secure `flatpak-spawn --host` bridge to inspect host `/proc` files and execute `pw-top`.
* **Zero Host Dependencies:** Requires zero host runtimes or external interpreted scripts.
* **Multi-Threaded Event Loop:** Employs `std::sync::mpsc` channels to keep the GTK4 UI rendering smoothly at 30 FPS while background threads poll hardware metrics every 500ms.

---

## 📄 License

This project is licensed under the **MIT License** — see the [LICENSE](LICENSE) file for details.

---

## 🙏 Acknowledgements & Disclosures

* **[gtk-rs](https://gtk-rs.org/)** – Used for building the native Linux user interface.
* **`pw-top` Visualizer** – Inspired by the original Python prototype.
* **AI Disclosure** – Developed with assistance from AI models (Gemini and DeepSeek).

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## 🙏 Acknowledgements & Disclosures

* **[gtk-rs](https://gtk-rs.org/)** – Used for building the native Linux user interface.
* **pw-top Visualizer** – Inspired by the original Python prototype.
* **AI Disclosure** – This project was developed with assistance from AI models (Gemini and DeepSeek).
