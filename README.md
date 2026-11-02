<div align="center">

# 🎧 HiFi Verify
### *A native PipeWire audio verification and telemetry suite for Linux audiophiles*

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Toolkit: GTK4](https://img.shields.io/badge/Toolkit-GTK4-blue.svg)](https://gtk-rs.org/)
[![Packaging: Flatpak](https://img.shields.io/badge/Packaging-Flatpak-blueviolet.svg)](https://flatpak.org/)

</div>

---

**HiFi Verify** is a high-performance, container-sandboxed utility designed to provide absolute transparency into the Linux audio stack. By bypassing high-level abstractions and reading directly from the underlying hardware interfaces, it gives you definitive proof of what your DAC is receiving—guaranteeing true, uncompromised bit-perfect playback.

---

## 🛠️ Key Architectural Features

* **Hardware-Direct Telemetry:** Bypasses PipeWire abstractions to parse `/proc/asound/card*/pcm*p/sub*/hw_params` directly, revealing the exact active sample rate and ALSA format (e.g., `S32_LE`) processed by your hardware.
* **Deterministic Bit-Perfect Audit:** Compares source stream parameters against ALSA hardware rates in real time, delivering instant validation (`✓ ALSA Bit-Perfect` vs. `⚠ ALSA Resampled`).
* **Graph & DSP Inspection:** Scans the active PipeWire graph to detect background processing nodes (such as EasyEffects, Carla, or JamesDSP) that alter bit-streams.
* **Low-Overhead Performance:** Written in **Rust**, achieving sub-40ms CLI startup times, a ~40MB idle RAM footprint, and multi-threaded event handling via `std::sync::mpsc` channels (maintaining 30 FPS UI rendering while background threads poll hardware every 500ms).
* **Flatpak Sandbox Integration:** Runs securely inside a Flatpak container while utilizing a controlled `flatpak-spawn --host` bridge to safely query host proc files and run `pw-top`.

---

## 🔬 Core Telemetry & Diagnostics

* **Signal Path Matrix:** A custom Cairo-rendered flowchart mapping your end-to-end audio route:
  $$\text{Source Application} \longrightarrow \text{Active DSP / Filters} \longrightarrow \text{Final ALSA DAC Bus}$$
* **Latency & Jitter Graphing:** Plots real-time `pw-top` WAIT metrics (in microseconds) to help spot buffer underruns and system jitter before they manifest as audible dropouts.
* **Hardware XRUN Tracking:** Monitors per-node dropped frames and logs them into a local SQLite database (`~/.config/hifi-verify/history.db`) for post-session analysis.
* **DAC Capability Viewer:** Queries hardware endpoints to expose device capabilities, including supported sample rates (8 kHz to 384 kHz), human-readable PCM formats, and Native DSD transport profiles.

---

## 💻 Graphical User Interface (GTK4)

* **Modern Dark-Mode Dashboard:** A sleek, monospace-styled interface tailored for technical workflows and power users.
* **Cava Audio Spectrum:** An integrated, zero-latency 32-band frequency visualizer driven by the `cava` backend, toggleable directly from the Signal Path view.
* **Hardware Volume Control:** Bypasses software attenuation by using `wpctl` to adjust hardware DAC levels directly, complete with bi-directional synchronization to system volume keys.
* **Live "Now Playing" Bar:** Displays active source applications, track metadata, and container codecs at a glance.
* **Quick-Launch Actions:** One-click shortcuts to open EasyEffects, KDE Audio Settings, or `qpwgraph`.

---

## 🖥️ System Integration & Tray Control

* **Dynamic Tray Icon (`StatusNotifierItem`):** Procedurally generated indicator reflecting real-time pipeline status:
  * 🟢 **Emerald Green:** Bit-Perfect stream active
  * 🔴 **Ruby Red:** Compromised / DSP processing active
  * ⚫ **Slate Grey:** Standby / Idle
* **Scroll-Wheel Volume:** Hover over the system tray icon and scroll to modify hardware volume in precise 5% increments.
* **Desktop Notifications:** Fires immediate system warnings if a DSP node is activated mid-playback.
* **KDE Plasma Widget (QML):** A lightweight Plasmoid that polls the background CLI service to display a clean status pill on your panel.
* **MPRIS & Media Keys:** Full integration with standard Linux media keys (`⏮ Previous`, `⏯ Play/Pause`, `⏭ Next`).

---

## 🌐 Network & Renderer Discovery

* **SSDP / UPnP Scanner:** Automatically discovers network-attached audio renderers, smart streamers, and Raspberry Pi endpoints across your local subnet.
* **Device Resolution:** Fetches and parses UPnP device description XML to present human-readable target names alongside their network addresses.

---

## ⌨️ Command Line Interface (CLI)

The companion CLI provides scriptable diagnostics and headless integration capabilities:

| Option | Description |
| :--- | :--- |
| `--health` | Returns a sub-100ms one-line summary of current audio pipeline health. |
| `--audit` | Outputs a comprehensive JSON dump of hardware parameters, active nodes, and active warnings. |
| `--audit --fast` | Performs a near-instant audit by skipping `pw-top` parsing overhead. |
| `--hw` | Enumerates detailed ALSA hardware capabilities for all registered PCM devices. |
| `--status` | Outputs strict JSON formatted for external status bars and widgets (`{"state": "green", "reason": "Bit-Perfect"}`). |
| `--network-scan` | Executes an active SSDP broadcast scan directly from the terminal. |
| `--history` | Queries and prints historical XRUN dropouts from the SQLite log. |
| `--ramp-test` | Runs a mathematical null-test by generating a reference WAV, routing it through a loopback, and verifying bit-for-bit transmission. |
| `--wireplumber` | Generates a persistent WirePlumber configuration rule matching your current optimal hardware rate and quantum. |

---

## 📄 License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for more information.

---

## 🙏 Acknowledgements

* Built with [gtk-rs](https://gtk-rs.org/) for native Linux UI performance.
* `pw-top` telemetry concepts inspired by earlier community utility scripts.
