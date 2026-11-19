<div align="center">

# 🎧 HiFi Verify
### *A native PipeWire & ALSA audio path verification and telemetry suite for Linux*

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Toolkit: GTK4](https://img.shields.io/badge/Toolkit-GTK4-blue.svg)](https://gtk-rs.org/)
[![Packaging: Flatpak](https://img.shields.io/badge/Packaging-Flatpak-blueviolet.svg)](https://flatpak.org/)

</div>

---

**HiFi Verify** provides transparent, real-time inspection of the Linux PipeWire and ALSA audio pipeline. It reads active playback hardware parameters directly from `/proc/asound/card*/pcm*p/sub*/hw_params`, traces the live PipeWire graph from the playing application to the hardware DAC, and distinguishes clearly between **observed PCM path cleanliness** and **proven sample-for-sample identity**.

---

## 🛠️ Key Architectural Features

* **Hardware-Direct ALSA Telemetry:** Parses `/proc/asound/card*/pcm*p/sub*/hw_params` directly to report the exact active playback sample rate, format (e.g., `S32_LE`), channels, and buffer size.
* **Route-Aware PipeWire Inspection:** Traces direct and multi-hop links in `pw-dump` combined with negotiated `pw-top` stream formats—flagging active DSP/filter nodes (EasyEffects, JamesDSP, Carla, filter-chain) when they are actually on the playback route, without falsely red-flagging unrelated background nodes.
* **Honest Verification Semantics:** Clearly separates *observed* PCM path status (`clear` / `altered` / `unknown` / `idle`) from true bit-exact sample verification (`--verify-reference` / `--verify-capture` and `--ramp-test`).
* **Real-Time Passive Stereo VU Meter:** Measures true Left/Right RMS, peak-hold, and digital clip indicators (`-60` to `0 dBFS`) via a passive PipeWire sink monitor (`node.passive=true`) that never wakes idle sinks or pins your DAC clock rate.
* **Flatpak Sandbox Support:** Uses a timeout-enforced `flatpak-spawn --host` bridge for PipeWire, ALSA proc, `pw-record`, and `cava` without deadlocking or leaking child processes.

---

## 🔬 Core Telemetry & Diagnostics

* **Signal Path Matrix:** Custom Cairo flowchart mapping `Source → DSP / Processing → ALSA DAC Bus` with color-coded path status.
* **Latency & Jitter Scope:** Plots live `pw-top` `WAIT` latency across a 60-second window.
* **Per-Node XRUN History:** Logs newly observed per-node XRUN increments to SQLite (`~/.config/hifi-verify/history.db`).
* **DAC Capability Viewer:** Parses USB Audio playback capabilities (`stream0`, including continuous rate ranges and Native DSD) with active ALSA fallback for PCI/HDA/HDMI hardware.
* **WirePlumber 0.5 Rule Generator:** Outputs valid SPA-JSON rules for `~/.config/wireplumber/wireplumber.conf.d/50-hifi-verify-alsa.conf`.

---

## 💻 Graphical User Interface (GTK4) & Configurable Shortcuts

* **Compact Dark Dashboard:** Monospace GTK4 interface with live DSP load gauge, source/stream/DAC format readout, buffer quantum & latency, output rate, session XRUN counter, and real stereo VU meter.
* **On-Demand Cava Spectrum:** 32-band frequency visualizer that runs only while toggled visible and shuts down cleanly on hide/exit.
* **Desktop-Agnostic, User-Configurable Shortcuts:** Launchers for Music Player, Audio Settings, DSP/EQ, and Patchbay automatically detect installed desktop tools (KDE, GNOME, `pwvucontrol`, `pavucontrol`, `easyeffects`, `qpwgraph`, `helvum`, etc.) and can be customized via **Devices & tools → Configure Shortcuts** (`~/.config/hifi-verify/config.json`).

---

## 🖥️ System Tray & KDE Plasma 6 Panel Widget

* **HiDPI Dynamic Tray Icon (`StatusNotifierItem`):** Procedural 32×32 and 64×64 badge with unmistakable status glyphs and a cyan Hi-Res pip (`>= 88.2 kHz`):
  * 🟢 **Emerald Check:** Direct PCM path clear (rates & format matched)
  * 🔴 **Ruby `!`:** Alteration detected (resampling, precision reduction, or DSP in route)
  * 🟡 **Amber `?`:** Unverified / ambiguous route
  * ⚫ **Slate Dot:** Standby / Idle
* **At-a-Glance Toolbar Readout:** Both the tray title/tooltip and the included **KDE Plasma 6 Panel Widget** (`plasmoid/org.cachyos.hifiverify/`) display live status, active sample rate, and DAC format directly in your panel.

Install the KDE Plasma 6 widget locally with:

```bash
kpackagetool6 --type Plasma/Applet --install plasmoid/org.cachyos.hifiverify
```

---

## ⌨️ Command Line Interface (CLI)

| Option | Description |
| :--- | :--- |
| `--health` | One-line summary of current codec/app, output rate, quantum, and warning count. |
| `--status` | JSON status for the Plasma widget and status bars (`state`, `status`, `badge`, `short_label`, `reason`, `app`, `stream_pcm`, `dac_pcm`, `rate_hz`, `quantum`, `route_dsp`). |
| `--audit` | Full JSON audit including hardware params, active route, findings, and suggested `fixes`. |
| `--audit --fast` | Skips XRUN reporting while keeping live `pw-top` PCM route observation. |
| `--hw` | Lists open ALSA playback PCM devices (`format`, `rate`, `channels`, `buffer_size`). |
| `--history` | Prints the last 50 logged per-node XRUN events from SQLite. |
| `--network-scan` | Scans the local subnet for UPnP / SSDP MediaRenderer devices and resolves friendly names. |
| `--verify-reference <WAV> --verify-capture <WAV>` | Performs strict sample-for-sample PCM identity comparison between a reference WAV and a captured monitor WAV (supports 16/24/32-bit lossless width expansion). |
| `--ramp-test` | Generates a 16-bit ramp WAV, plays it through a detected loopback/monitor sink, and verifies capture via `pcm_verify`. |
| `--wireplumber` | Prints a WirePlumber 0.5 `.conf` rule matching the active driver rate and quantum. |
| `--gui` | Launches the GTK4 desktop dashboard and system tray (default when no CLI flag is passed). |

---

## 📄 License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for details.
